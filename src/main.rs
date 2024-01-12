#![no_std]
#![no_main]

// use panic_halt as _;
use esp_backtrace as _;

use heapless::String;

use esp32c3_hal::{
    clock::ClockControl,
    gpio::IO,
    peripherals::Peripherals,
    prelude::*,
    Rng,
    spi::{master::Spi, SpiMode},
    systimer::SystemTimer,
    timer::TimerGroup,
    Delay,
    Rtc,
};

use embedded_svc::{
    io::{Read, Write},
    ipv4::Interface,
    wifi::{AccessPointInfo, ClientConfiguration, Configuration, Wifi},
};
use esp_wifi::{
    current_millis, EspWifiInitFor, initialize,
    wifi::{
        {WifiError, WifiStaDevice},
        utils::create_network_interface,
    },
    wifi_interface::WifiStack,
};
use smoltcp::{
    iface::SocketStorage,
    wire::{IpAddress, Ipv4Address},
};

use esp_println::{print, println};

// module used for isolating core::fmt::Write that conflicts with embedded_io::blocking::Write
// used in esp_wifi::wifi_interface
mod isolated_write {
    use core::fmt::Write;
    use heapless::String;

    pub fn write_http_request_string(s: &mut String<128>, temp: f32) -> () {
        s.truncate(0);
        write!(s, "GET /temp/1/{:.2} HTTP/1.0\r\nHost: 192.168.1.103:8080\r\n\r\n", temp).unwrap();
    }
}

const SSID: &str = "XXXXXXXXXXXX";
const PASSWORD: &str = "XXXXXXXXXXXXXXXXXXXXXXXXXX";

#[entry]
fn main() -> ! {
    let peripherals = Peripherals::take();
    let system = peripherals.SYSTEM.split();
    let clocks = ClockControl::max(system.clock_control).freeze();

    // Disable the watchdog timers. For the ESP32-C3, this includes the Super WDT,
    // the RTC WDT, and the TIMG WDTs.
    let mut rtc = Rtc::new(peripherals.RTC_CNTL);
    let timer_group0 = TimerGroup::new(
        peripherals.TIMG0,
        &clocks,
    );
    let mut wdt0 = timer_group0.wdt;
    let timer_group1 = TimerGroup::new(
        peripherals.TIMG1,
        &clocks,
    );
    let mut wdt1 = timer_group1.wdt;

    rtc.swd.disable();
    rtc.rwdt.disable();
    wdt0.disable();
    wdt1.disable();

    // wifi
    let timer = SystemTimer::new(peripherals.SYSTIMER).alarm0;
    let init = initialize(
        EspWifiInitFor::Wifi,
        timer,
        Rng::new(peripherals.RNG),
        system.radio_clock_control,
        &clocks,
    ).unwrap();

    let wifi = peripherals.WIFI;
    let mut socket_set_entries: [SocketStorage; 3] = Default::default();
    let (iface, device, mut controller, sockets) =
        create_network_interface(&init, wifi, WifiStaDevice, &mut socket_set_entries).unwrap();
    let wifi_stack = WifiStack::new(iface, device, sockets, current_millis);

    let client_config = Configuration::Client(ClientConfiguration {
        ssid: SSID.into(),
        password: PASSWORD.into(),
        ..Default::default()
    });
    let res = controller.set_configuration(&client_config);
    println!("wifi_set_configuration returned {:?}", res);

    controller.start().unwrap();
    println!("is wifi started: {:?}", controller.is_started());

    println!("Start Wifi Scan");
    let res: Result<(heapless::Vec<AccessPointInfo, 10>, usize), WifiError> = controller.scan_n();
    if let Ok((res, _count)) = res {
        for ap in res {
            println!("{:?}", ap);
        }
    }

    println!("{:?}", controller.get_capabilities());
    println!("wifi_connect {:?}", controller.connect());

    // wait to get connected
    println!("Wait to get connected");
    loop {
        let res = controller.is_connected();
        match res {
            Ok(connected) => {
                if connected {
                    break;
                }
            }
            Err(err) => {
                println!("{:?}", err);
                loop {}
            }
        }
    }
    println!("{:?}", controller.is_connected());

    // wait for getting an ip address
    println!("Wait to get an ip address");
    loop {
        wifi_stack.work();

        if wifi_stack.is_iface_up() {
            println!("got ip {:?}", wifi_stack.get_ip_info());
            break;
        }
    }

    println!("Start busy loop on main");

    let mut rx_buffer = [0u8; 1536];
    let mut tx_buffer = [0u8; 1536];
    let mut socket = wifi_stack.get_socket(&mut rx_buffer, &mut tx_buffer);

    // led
    // Set GPIO8 as an output, and set its state high initially.
    let io = IO::new(peripherals.GPIO, peripherals.IO_MUX);

    let mut led = io.pins.gpio8.into_push_pull_output();
    led.set_high().unwrap();

    // max6675
    let cs = io.pins.gpio10;
    let sclk = io.pins.gpio6;
    let miso = io.pins.gpio2;
    let mosi = io.pins.gpio7;

    let mut spi = Spi::new(peripherals.SPI2, 100u32.kHz(), SpiMode::Mode0, &clocks).with_pins(
        Some(sclk),
        Some(mosi),
        Some(miso),
        Some(cs),
    );

    // Initialize the Delay peripheral, and use it to toggle the LED state in a
    // loop.
    let mut delay = Delay::new(&clocks);

    let mut data = [0u8; 2];
    let mut request: String<128> = String::new();
    loop {
        // get temperature from max6675
        spi.transfer(&mut data).unwrap();
        let mut temp = u16::from_be_bytes(data[..].try_into().unwrap());
        if temp & 4 == 1 {
            println!("No sensor attached");
            delay.delay_ms(500u32);
            continue;
        }
        temp = temp >> 3;
        let temp = temp as f32 * 0.25;
        println!("temperature = {}°C", temp);

        // toggle led
        led.toggle().unwrap();

        println!("Making HTTP request");
        isolated_write::write_http_request_string(&mut request, temp);
        socket.work();
        socket.open(IpAddress::Ipv4(Ipv4Address::new(192, 168, 1, 103)), 8080).unwrap();
        socket.write(request.as_bytes()).unwrap();
        socket.flush().unwrap();

        let wait_end = current_millis() + 20 * 1000;
        loop {
            let mut buffer = [0u8; 512];
            if let Ok(len) = socket.read(&mut buffer) {
                let to_print = unsafe { core::str::from_utf8_unchecked(&buffer[..len]) };
                print!("{}", to_print);
            } else {
                break;
            }

            if current_millis() > wait_end {
                println!("Timeout");
                break;
            }
        }
        println!();

        socket.disconnect();

        let wait_end = current_millis() + 5 * 1000;
        while current_millis() < wait_end {
            socket.work();
        }
    }
}
