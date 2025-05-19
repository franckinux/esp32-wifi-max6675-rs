#![no_std]
#![no_main]


use core::net::Ipv4Addr;

// use fugit::rate::ExtU32;
use heapless::String;
use blocking_network_stack::Stack;
use embedded_io::*;
use esp_alloc as _;
use esp_backtrace as _;
use esp_hal::{
    clock::CpuClock,
    delay::Delay,
    gpio::{Input, InputConfig, Level, Output, OutputConfig, Pull},
    main,
    rng::Rng,
    spi::{master, Mode},
    time::{self, Duration, Rate},
    timer::timg::TimerGroup,
};
use esp_println::{print, println};
use esp_wifi::{
    init,
    wifi::{ClientConfiguration, Configuration},
};
use smoltcp::{
    iface::{SocketSet, SocketStorage},
    wire::{DhcpOption, IpAddress},
};

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

const SSID: &str = env!("SSID");
const PASSWORD: &str = env!("PASSWORD");

#[main]
fn main() -> ! {
    esp_println::logger::init_logger_from_env();
    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    esp_alloc::heap_allocator!(size: 72 * 1024);

    let timg0 = TimerGroup::new(peripherals.TIMG0);

    let mut rng = Rng::new(peripherals.RNG);

    let esp_wifi_ctrl = init(timg0.timer0, rng.clone(), peripherals.RADIO_CLK).unwrap();

    let (mut controller, interfaces) =
        esp_wifi::wifi::new(&esp_wifi_ctrl, peripherals.WIFI).unwrap();

    let mut device = interfaces.sta;
    let iface = create_interface(&mut device);

    let mut socket_set_entries: [SocketStorage; 3] = Default::default();
    let mut socket_set = SocketSet::new(&mut socket_set_entries[..]);
    let mut dhcp_socket = smoltcp::socket::dhcpv4::Socket::new();
    // we can set a hostname here (or add other DHCP options)
    dhcp_socket.set_outgoing_options(&[DhcpOption {
        kind: 12,
        data: b"esp-wifi",
    }]);
    socket_set.add(dhcp_socket);

    let now = || time::Instant::now().duration_since_epoch().as_millis();
    let stack = Stack::new(iface, device, socket_set, now, rng.random());

    controller
        .set_power_saving(esp_wifi::config::PowerSaveMode::None)
        .unwrap();

    let client_config = Configuration::Client(ClientConfiguration {
        ssid: SSID.try_into().unwrap(),
        password: PASSWORD.try_into().unwrap(),
        ..Default::default()
    });
    let res = controller.set_configuration(&client_config);
    println!("wifi_set_configuration returned {:?}", res);

    controller.start().unwrap();
    println!("is wifi started: {:?}", controller.is_started());

    println!("Start Wifi Scan");
    let res = controller.scan_n::<10>();
    if let Ok((res, _count)) = res {
        for ap in res {
            println!("{:?}", ap);
        }
    }

    println!("{:?}", controller.capabilities());
    println!("wifi_connect {:?}", controller.connect());

    // wait to get connected
    println!("Wait to get connected");
    loop {
        match controller.is_connected() {
            Ok(true) => break,
            Ok(false) => {}
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
        stack.work();

        if stack.is_iface_up() {
            println!("got ip {:?}", stack.get_ip_info());
            break;
        }
    }

    println!("Start busy loop on main");

    let mut rx_buffer = [0u8; 1536];
    let mut tx_buffer = [0u8; 1536];
    let mut socket = stack.get_socket(&mut rx_buffer, &mut tx_buffer);

    // led
    // Set GPIO8 as an output, and set its state high initially.
    let mut led = Output::new(peripherals.GPIO8, Level::High, OutputConfig::default());

    // max6675
    let cs = Output::new(peripherals.GPIO10, Level::Low, OutputConfig::default());
    let sclk = Output::new(peripherals.GPIO6, Level::Low, OutputConfig::default());
    let miso = Input::new(peripherals.GPIO2, InputConfig::default().with_pull(Pull::None));
    let mosi = Output::new(peripherals.GPIO7, Level::Low, OutputConfig::default());

    let config = master::Config::default().with_frequency(Rate::from_khz(100)).with_mode(Mode::_0);
    let mut spi = master::Spi::new(peripherals.SPI2, config).unwrap()
        .with_sck(sclk).with_mosi(mosi).with_miso(miso).with_cs(cs);

    // Initialize the Delay peripheral, and use it to toggle the LED state in a
    // loop.
    let delay = Delay::new();

    let mut data = [0u8; 2];
    let mut request: String<128> = String::new();
    loop {
        // get temperature from max6675
        spi.transfer(&mut data).unwrap();
        let mut temp = u16::from_be_bytes(data[..].try_into().unwrap());
        if temp & 4 == 1 {
            println!("No sensor attached");
            delay.delay_millis(500u32);
            continue;
        }
        temp = temp >> 3;
        let temp = temp as f32 * 0.25;
        println!("temperature = {}°C", temp);

        // toggle led
        led.toggle();

        println!("Making HTTP request");
        isolated_write::write_http_request_string(&mut request, temp);
        socket.work();

        socket
            .open(IpAddress::Ipv4(Ipv4Addr::new(192, 168, 1, 103)), 8080)
            .unwrap();

        socket
            .write(request.as_bytes())
            .unwrap();
        socket.flush().unwrap();

        let deadline = time::Instant::now() + Duration::from_secs(20);
        let mut buffer = [0u8; 512];
        while let Ok(len) = socket.read(&mut buffer) {
            let to_print = unsafe { core::str::from_utf8_unchecked(&buffer[..len]) };
            print!("{}", to_print);

            if time::Instant::now() > deadline {
                println!("Timeout");
                break;
            }
        }
        println!();

        socket.disconnect();

        let deadline = time::Instant::now() + Duration::from_secs(5);
        while time::Instant::now() < deadline {
            socket.work();
        }
    }
}

// some smoltcp boilerplate
fn timestamp() -> smoltcp::time::Instant {
    smoltcp::time::Instant::from_micros(
        esp_hal::time::Instant::now()
            .duration_since_epoch()
            .as_micros() as i64,
    )
}

pub fn create_interface(device: &mut esp_wifi::wifi::WifiDevice) -> smoltcp::iface::Interface {
    // users could create multiple instances but since they only have one WifiDevice
    // they probably can't do anything bad with that
    smoltcp::iface::Interface::new(
        smoltcp::iface::Config::new(smoltcp::wire::HardwareAddress::Ethernet(
            smoltcp::wire::EthernetAddress::from_bytes(&device.mac_address()),
        )),
        device,
        timestamp(),
    )
}
