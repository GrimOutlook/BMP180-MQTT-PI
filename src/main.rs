use bmp085::*;
use clap::Parser;
use i2cdev::linux::*;
use i2cdev::sensors::{Barometer, Thermometer};
use log::{info, debug, error};
use rumqttc::{Client,Connection,Event,Incoming,MqttOptions,QoS};
use secrecy::{ExposeSecret, SecretBox};
use serde_derive::Deserialize;
use std::error::Error;
use std::{fs, thread};
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

#[derive(Deserialize)]
struct Data {
    mqtt_broker: MQTTBroker,
    mqtt: MQTT,
    logging: Logging,
}

#[derive(Deserialize)]
struct MQTTBroker {
    host: String,
    port: u16,
    username: String,
    password: SecretBox<String>,
}

#[derive(Deserialize)]
struct MQTT {
    room: String,
    identifier: String,
    name: String,
}

#[derive(Deserialize)]
struct Logging {
    log_level: Option<String>,
}

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    #[arg(short, long)]
    log_level: Option<String>,

    #[arg(short, long, default_value_t = String::from("bmp180.toml") )]
    config: String
}

enum SensorComponent {
    Temperature,
    Pressure
}

fn main() -> ExitCode {

    // Read passed in arguments
    let args = Args::parse();

    // Read config data
    let config: Data = match read_config(PathBuf::from(&args.config)) {
        Ok(c) => c,
        Err(e) => return e
    };

    // Init logging
    init_logging(args, &config);

    info!("Starting BMP180 Temperature/Pressure Sensor");

    let i2c_dev = match LinuxI2CDevice::new("/dev/i2c-1", BMP085_I2C_ADDR) {
        Ok(x) => x,
        Err(_) => {
            error!("Cannot initialize I2C device.");
            return ExitCode::FAILURE
        },
    };

    let Ok(sensor) = BMP085BarometerThermometer::new(i2c_dev, SamplingMode::UltraHighRes) else {
        error!("Can't initialize PMB180 thermostat sensor");
        return ExitCode::FAILURE;
    };

    let (client, connection) =  get_mqtt_client(&config);

    thread::spawn(move || {read_and_publish_data(sensor, client, config)});
    poll_for_events(connection);

    return ExitCode::SUCCESS;
}

fn read_and_publish_data(mut sensor: BMP085BarometerThermometer<LinuxI2CDevice>, client: Client, config: Data) -> ExitCode {
    info!("Starting read and publish thread");
    
    loop {
        thread::sleep(Duration::from_secs(1));
        let Ok((temp, pressure)) = read_from_sensor(&mut sensor) else {
            error!("Cannot initialize I2C device.");
            return ExitCode::FAILURE;
        };

        match publish_sensor_data(&client, &config, temp, pressure) {
            Ok(_) => (),
            Err(_) => {
                return ExitCode::FAILURE
            }
        };
    }
}

fn poll_for_events(mut connection: Connection) {
    loop {
        debug!("Polling for events");
        for notification in connection.iter() {
            match notification {
                Ok(Event::Incoming(Incoming::Connect(c))) => debug!("Connected to MQTT broker {}", c.client_id),
                Ok(e) => {
                    debug!("Got event: {:?}", e);
                },
                Err(e) => {
                    error!("Got an error when polling for events: {}", e.to_string());
                },
            }
        }
    }
}

fn init_logging(args: Args, config: &Data) {
    let log_level = args.log_level.unwrap_or(
        config.logging.log_level.clone().unwrap_or(
            String::from("Info")
        ));

    env_logger::builder().filter_level(log_level.parse().unwrap()).init();
}

fn read_config(config: PathBuf) -> Result<Data, ExitCode> {
    let contents = match fs::read_to_string(&config) {
        Ok(c) => c,
        Err(_) => {
            error!("Could not read file `{}`", config.to_str().unwrap());
            return Err(ExitCode::FAILURE);
        }
    };

    let data: Data = match toml::from_str(&contents) {
        Ok(d) => d,
        Err(e) => {
            error!("Unable to load data from `{}` due to error: {}", config.to_str().unwrap(), e.message());
            return Err(ExitCode::FAILURE);
        }
    };

    Ok(data)
}

fn get_mqtt_client(config: &Data) -> (Client, Connection) {
    let mut mqttoptions = MqttOptions::new(&config.mqtt.name, &config.mqtt_broker.host, config.mqtt_broker.port);
    mqttoptions.set_keep_alive(Duration::from_secs(5));
    mqttoptions.set_credentials(&config.mqtt_broker.username, config.mqtt_broker.password.expose_secret());

    let (client, connection) = Client::new(mqttoptions, 10);

    (client, connection)
}

fn read_from_sensor(sensor: &mut BMP085BarometerThermometer<LinuxI2CDevice>) -> Result<(f32, f32), Box<dyn Error>> {
    let temp = sensor.temperature_celsius()?;
    let pressure = sensor.pressure_kpa()?;

    debug!("Read sensor data. Temp: [{}]. Pressure: [{}].", temp, pressure);
    Ok((temp, pressure))
}

fn publish_sensor_data(client: &Client, config: &Data, temp: f32, pressure: f32) -> Result<(), ExitCode> {
    publish_temperature(client, config, temp)?;
    publish_pressure(client, config, pressure)?;
    return Ok(());
}

fn publish_temperature(client: &Client, config: &Data, temp: f32) -> Result<(), ExitCode> {
    let topic = format!("homeassistant/sensor/{}Temperature/config", config.mqtt.room);
    debug!("Publishing sensor data to topic [{}]", topic);
    let msg = get_message(config, SensorComponent::Temperature, temp);
    match client.publish(topic, QoS::AtMostOnce, true, msg) {
        Ok(_) => return Ok(()),
        Err(e) => {
            error!("Failed to publish temerature due to error: {}", e);
            return Err(ExitCode::FAILURE);
        }
    };
}

fn publish_pressure(client: &Client, config: &Data, pressure: f32) -> Result<(), ExitCode> {
    let topic = format!("homeassistant/sensor/{}Pressure/config", config.mqtt.room);
    debug!("Publishing sensor data to topic [{}]", topic);
    let msg = get_message(config, SensorComponent::Pressure, pressure);
    match client.publish(topic, QoS::AtMostOnce, true, msg) {
        Ok(_) => return Ok(()),
        Err(e) => {
            error!("Failed to publish pressure due to error: {}", e);
            return Err(ExitCode::FAILURE);
        }
    };
}

fn get_message(config: &Data, sensor_component: SensorComponent, temp: f32) -> String {
    let sensor_component_str = match sensor_component {
        SensorComponent::Temperature => "temperature",
        SensorComponent::Pressure => "pressure"
    };

    let temperature_msg = format!("\
{{  
   \"device_class\":\"{1}\",
   \"state_topic\":\"homeassistant/sensor/{2}/state\",
   \"unit_of_measurement\":\"°C\",
   \"value_template\":\"{0}\",
   \"unique_id\":\"{1}\",
   \"device\":{{
      \"identifiers\":[
          \"{3}\"
      ],
      \"name\":\"{4}\"
    }}
}}
",
    temp,
    sensor_component_str,
    config.mqtt.room,
    config.mqtt.identifier,
    config.mqtt.name);

    return temperature_msg;
}