Instructions from https://github.com/esp-rs/rust-build.

Installation (to be verified)
=============================

.. code:: console

        rustup target add riscv32imc-unknown-none-elf

        apt-get install -y git curl gcc clang ninja-build cmake libudev-dev unzip xz-utils \
        python3 python3-pip python3-venv libusb-1.0-0 libssl-dev pkg-config libpython2.7

        cargo install espup
        espup install

        cargo install cargo-generate
        cargo install ldproxy
        cargo install espup
        sudo apt install libudev-dev
        cargo install espflash
        cargo install cargo-espmonitor

Project creation
================

.. code:: console

        cargo generate esp-rs/esp-template

Building and flashing
=====================

.. code:: console

        cargo build
        cargo run

Just monitoring :

.. code:: console

    cargo espmonitor -c esp32c3 /dev/ttyACM0
