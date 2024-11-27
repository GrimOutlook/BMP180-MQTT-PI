#!/bin/bash
#
# Shamelessly stolen from this medium article on cross-compiling from an x86_64 Linux system to a Raspberry Pi.

set -o errexit
set -o nounset
set -o pipefail
set -o xtrace

readonly TARGET_HOST=Ottowa
readonly TARGET_PATH=/home/ottowa/BMP180-MQTT
readonly TARGET_ARCH=aarch64-unknown-linux-gnu
readonly SOURCE_PATH=./target/${TARGET_ARCH}/release/BMP180-MQTT

cargo build --release --target=${TARGET_ARCH}
rsync ${SOURCE_PATH} ${TARGET_HOST}:${TARGET_PATH}
ssh -t ${TARGET_HOST} ${TARGET_PATH} --log-level DEBUG
