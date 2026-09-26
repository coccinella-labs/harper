// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 coccinella-labs

use harper_firmware::{Esp32Device, FirmwareDevice, FirmwareRegistry};

fn main() {
    println!("Harper Firmware Crate Test\n");

    let esp = Esp32Device::new("/dev/ttyUSB0", "my-esp32");
    let info = esp.device_info();

    println!("Device: {}", info.name);
    println!("Platform: {:?}", info.platform);
    println!("Capabilities: {:?}", info.capabilities);

    let mut registry = FirmwareRegistry::new();
    registry.register("esp32-1".to_string(), Box::new(esp));

    println!("\nRegistered devices: {:?}", registry.list_devices());
    println!("\n✓ harper-firmware crate is working!");
}
