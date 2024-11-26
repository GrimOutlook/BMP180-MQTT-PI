use bmp085::*;
use clap::Parser;
use i2cdev::linux::*;
use i2cdev::sensors::{Barometer, Thermometer};
use log::{info, debug, LevelFilter, error, Level};
use rumqttc::{MqttOptions, AsyncClient, QoS};
use secrecy::{ExposeSecret, SecretBox};
use serde_derive::Deserialize;
use std::error::Error;
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;
use tokio::time;

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
    id: String,
    temperature_topic: String,
    pressure_topic: String,
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

#[tokio::main]
async fn main() -> Result<(), ExitCode> {
    info!("Starting BMP180 Temperature/Pressure Sensor");

    // Read passed in arguments
    let args = Args::parse();

    // Read config data
    let config: Data = read_config(PathBuf::from(&args.config))?;

    // Init logging
    init_logging(args, &config);

    let i2c_dev = match LinuxI2CDevice::new("/dev/i2c-1", BMP085_I2C_ADDR) {
        Ok(x) => x,
        Err(_) => {
            let msg = "Cannot initialize I2C device.";
            eprintln!("{}", msg);
            error!("{}", msg);
            return Err(ExitCode::FAILURE)
        },
    };

    let mut sensor = BMP085BarometerThermometer::new(i2c_dev, SamplingMode::UltraHighRes).unwrap();

    let client =  get_mqtt_client(&config);

    loop {
        let Ok((temp, pressure)) = read_from_sensor(&mut sensor) else {
            let msg = "Cannot initialize I2C device.";
            eprintln!("{}", msg);
            error!("{}", msg);
            return Err(ExitCode::FAILURE);
        };

        publish_sensor_data(&client, &config, temp, pressure).await;
        time::sleep(Duration::from_millis(100)).await;
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
            let msg = format!("Could not read file `{}`", config.to_str().unwrap());
            error!("{}", msg);
            eprintln!("{}", msg);
            return Err(ExitCode::FAILURE);
        }
    };

    let data: Data = match toml::from_str(&contents) {
        Ok(d) => d,
        Err(_) => {
            let msg = format!("Unable to load data from `{}`", config.to_str().unwrap());
            error!("{}", msg);
            eprintln!("{}", msg);
            return Err(ExitCode::FAILURE);
        }
    };

    Ok(data)
}

fn get_mqtt_client(config: &Data) -> AsyncClient {
    let mut mqttoptions = MqttOptions::new(&config.mqtt.id, &config.mqtt_broker.host, config.mqtt_broker.port);
    mqttoptions.set_keep_alive(Duration::from_secs(5));
    mqttoptions.set_credentials(&config.mqtt_broker.username, config.mqtt_broker.password.expose_secret());

    let (client, _) = AsyncClient::new(mqttoptions, 10);

    client
}

fn read_from_sensor(sensor: &mut BMP085BarometerThermometer<LinuxI2CDevice>) -> Result<(f32, f32), Box<dyn Error>> {
    let temp = sensor.temperature_celsius()?;
    let pressure = sensor.pressure_kpa()?;

    debug!("Read sensor data. Temp: [{}]. Pressure: [{}].", temp, pressure);
    Ok((temp, pressure))
}

async fn publish_sensor_data(client: &AsyncClient, config: &Data, temp: f32, pressure: f32) {
    debug!("Publishing sensor data to topic [{}]", &config.mqtt.topic);
    let temperature_msg = format!("{{\"temperature\": {0}}}", temp);
    let pressure_msg = format!("{{\"pressure\": {0}}}", pressure);
    client.publish(&config.mqtt.topic, QoS::AtLeastOnce, true, msg).await.unwrap();
}