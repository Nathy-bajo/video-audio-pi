readonly TARGET_HOST=pi@192.168.100.50
readonly TARGET_PATH=/home/pi/testing_tide
readonly TARGET_ARCH=armv7-unknown-linux-gnueabihf
readonly SOURCE_PATH=./target/${TARGET_ARCH}/release/testing_tide

cargo build --release --target=${TARGET_ARCH}
rsync -P ${SOURCE_PATH} ${TARGET_HOST}:${TARGET_PATH}

ssh ${TARGET_HOST} su -c "/home/pi/testing_tide"