//! Blinks an LED
//!
//! This assumes that a LED is connected to the pin assigned to `led`. (GPIO5)

#![no_std]
#![no_main]

use hal::{
    clock::ClockControl,
    gpio::IO,
    peripherals::Peripherals,
    prelude::*,
    spi,
    timer::TimerGroup,
    Delay,
    Rtc,
};

use core::panic::PanicInfo;

use esp_println::println;


#[panic_handler]
fn panic(_panic: &PanicInfo<'_>) -> ! {
    loop {}
}


#[entry]
fn main() -> ! {
    let peripherals = Peripherals::take();
    let mut system = peripherals.SYSTEM.split();
    let mut clocks = ClockControl::boot_defaults(system.clock_control).freeze();

    // Disable the watchdog timers. For the ESP32-C3, this includes the Super WDT,
    // the RTC WDT, and the TIMG WDTs.
    let mut rtc = Rtc::new(peripherals.RTC_CNTL);
    let timer_group0 = TimerGroup::new(
        peripherals.TIMG0,
        &clocks,
        &mut system.peripheral_clock_control,
    );
    let mut wdt0 = timer_group0.wdt;
    let timer_group1 = TimerGroup::new(
        peripherals.TIMG1,
        &clocks,
        &mut system.peripheral_clock_control,
    );
    let mut wdt1 = timer_group1.wdt;

    rtc.swd.disable();
    rtc.rwdt.disable();
    wdt0.disable();
    wdt1.disable();

    // Set GPIO5 as an output, and set its state high initially.
    let io = IO::new(peripherals.GPIO, peripherals.IO_MUX);

    // led
    let mut led = io.pins.gpio8.into_push_pull_output();
    led.set_high().unwrap();

    // max6675
    let cs = io.pins.gpio10;
    let sclk = io.pins.gpio6;
    let miso = io.pins.gpio2;
    let mosi = io.pins.gpio7;

    let mut spi = hal::spi::Spi::new(
        peripherals.SPI2,
        sclk,
        mosi,
        miso,
        cs,
        100u32.kHz(),
        spi::SpiMode::Mode0,
        &mut system.peripheral_clock_control,
        &mut clocks,
    );

    // Initialize the Delay peripheral, and use it to toggle the LED state in a
    // loop.
    let mut delay = Delay::new(&clocks);

    let mut data = [0u8; 2];
    loop {
        spi.transfer(&mut data).unwrap();
        let mut temp = u16::from_be_bytes(data[..].try_into().unwrap());
        // test temp & 04. == 1 if error
        if temp & 4 == 0 {
            temp = temp >> 3;
            println!("temperature = {}°C", temp as f64 * 0.25);
        } else {
            println!("no sensor attached");
        }

        led.toggle().unwrap();

        delay.delay_ms(500u32);
    }
}
