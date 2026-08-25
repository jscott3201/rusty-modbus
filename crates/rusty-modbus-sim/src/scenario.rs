//! Pre-built device profiles for common equipment types.

use crate::config::{
    CoilBlock, DeviceConfig, RegisterBlock, RegisterConfig, SimConfig, UpdateMode,
};

/// HVAC controller profile — typical BAS field device.
///
/// Holding registers: setpoints, PID parameters.
/// Input registers: temperature, humidity, pressure sensors.
/// Coils: fan on/off, damper open/close, valve enable.
#[must_use]
pub fn hvac_controller() -> SimConfig {
    SimConfig {
        device: DeviceConfig {
            unit_id: 1,
            vendor_name: String::from("Acme Controls"),
            product_code: String::from("ACM-HVAC-100"),
            revision: String::from("2.1.0"),
            listen_addr: String::from("127.0.0.1:0"),
        },
        registers: RegisterConfig {
            holding: vec![RegisterBlock {
                address: 0,
                count: 10,
                initial: vec![
                    720, // Setpoint: 72.0°F (×10)
                    680, // Heating setpoint
                    760, // Cooling setpoint
                    50,  // Fan speed %
                    100, // PID P gain (×10)
                    20,  // PID I gain (×10)
                    5,   // PID D gain (×10)
                    0, 0, 0,
                ],
                mode: UpdateMode::Static,
                min: 0,
                max: u16::MAX,
            }],
            input: vec![RegisterBlock {
                address: 0,
                count: 8,
                initial: vec![721, 450, 1013, 0, 0, 0, 0, 0],
                mode: UpdateMode::Static,
                min: 0,
                max: u16::MAX,
            }],
            coils: vec![CoilBlock {
                address: 0,
                count: 8,
                initial: vec![true, false, true, false, false, false, false, false],
            }],
            discrete_inputs: vec![],
        },
        faults: vec![],
    }
}

/// Power meter profile — energy monitoring device.
///
/// Input registers: voltage, current, power, energy counters.
#[must_use]
pub fn power_meter() -> SimConfig {
    SimConfig {
        device: DeviceConfig {
            unit_id: 2,
            vendor_name: String::from("PowerCo"),
            product_code: String::from("PM-3000"),
            revision: String::from("1.0.0"),
            listen_addr: String::from("127.0.0.1:0"),
        },
        registers: RegisterConfig {
            holding: vec![],
            input: vec![RegisterBlock {
                address: 0,
                count: 10,
                initial: vec![
                    2400, // Voltage L1 (×10 = 240.0V)
                    2390, // Voltage L2
                    2410, // Voltage L3
                    150,  // Current L1 (×10 = 15.0A)
                    148,  // Current L2
                    152,  // Current L3
                    3600, // Active power (W)
                    100,  // Power factor (×100 = 1.00)
                    0, 0,
                ],
                mode: UpdateMode::Static,
                min: 0,
                max: u16::MAX,
            }],
            coils: vec![],
            discrete_inputs: vec![],
        },
        faults: vec![],
    }
}

/// Variable frequency drive profile.
///
/// Holding registers: speed setpoint, accel/decel times.
/// Input registers: actual speed, motor current, bus voltage.
#[must_use]
pub fn vfd_drive() -> SimConfig {
    SimConfig {
        device: DeviceConfig {
            unit_id: 3,
            vendor_name: String::from("DriveTech"),
            product_code: String::from("VFD-500"),
            revision: String::from("3.2.1"),
            listen_addr: String::from("127.0.0.1:0"),
        },
        registers: RegisterConfig {
            holding: vec![RegisterBlock {
                address: 0,
                count: 6,
                initial: vec![
                    1500, // Speed setpoint (RPM)
                    10,   // Accel time (seconds)
                    10,   // Decel time (seconds)
                    0,    // Run command (0=stop, 1=run)
                    0, 0,
                ],
                mode: UpdateMode::Static,
                min: 0,
                max: u16::MAX,
            }],
            input: vec![RegisterBlock {
                address: 0,
                count: 4,
                initial: vec![0, 0, 480, 25],
                mode: UpdateMode::Static,
                min: 0,
                max: u16::MAX,
            }],
            coils: vec![CoilBlock {
                address: 0,
                count: 4,
                initial: vec![false, false, false, false],
            }],
            discrete_inputs: vec![CoilBlock {
                address: 0,
                count: 4,
                initial: vec![true, false, false, false],
            }],
        },
        faults: vec![],
    }
}

/// Generic I/O module — simple 16 coils + 16 registers.
#[must_use]
pub fn generic_io() -> SimConfig {
    SimConfig {
        device: DeviceConfig {
            unit_id: 1,
            vendor_name: String::from("Generic"),
            product_code: String::from("IO-16"),
            revision: String::from("1.0.0"),
            listen_addr: String::from("127.0.0.1:0"),
        },
        registers: RegisterConfig {
            holding: vec![RegisterBlock {
                address: 0,
                count: 16,
                initial: vec![0; 16],
                mode: UpdateMode::Static,
                min: 0,
                max: 65535,
            }],
            input: vec![RegisterBlock {
                address: 0,
                count: 16,
                initial: vec![0; 16],
                mode: UpdateMode::Static,
                min: 0,
                max: 65535,
            }],
            coils: vec![CoilBlock {
                address: 0,
                count: 16,
                initial: vec![false; 16],
            }],
            discrete_inputs: vec![CoilBlock {
                address: 0,
                count: 16,
                initial: vec![false; 16],
            }],
        },
        faults: vec![],
    }
}
