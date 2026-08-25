//! Simulator configuration validation tests.

use rusty_modbus_sim::config::{
    CoilBlock, FaultConfig, FaultTrigger, FaultType, RegisterBlock, UpdateMode,
};
use rusty_modbus_sim::{ModbusSimulator, SimConfig, SimError, generic_io};

fn register_block(address: u16, count: u16, initial: Vec<u16>) -> RegisterBlock {
    RegisterBlock {
        address,
        count,
        initial,
        mode: UpdateMode::Static,
        min: 0,
        max: u16::MAX,
    }
}

fn coil_block(address: u16, count: u16, initial: Vec<bool>) -> CoilBlock {
    CoilBlock {
        address,
        count,
        initial,
    }
}

fn config_error(config: SimConfig) -> String {
    match ModbusSimulator::from_config(config).unwrap_err() {
        SimError::Config(message) => message,
        other => panic!("expected semantic configuration error, got {other}"),
    }
}

#[test]
fn packaged_basic_example_validates() {
    ModbusSimulator::from_yaml(include_str!("../examples/basic.yaml")).unwrap();
}

#[test]
fn from_yaml_rejects_unknown_fields_at_each_struct_level() {
    let cases = [
        (
            "top level",
            "device: {}\nregisters: {}\nfaults: []\nextra: true\n",
        ),
        (
            "device",
            "device:\n  extra: true\nregisters: {}\nfaults: []\n",
        ),
        (
            "register tables",
            "device: {}\nregisters:\n  extra: []\nfaults: []\n",
        ),
        (
            "register block",
            "device: {}\nregisters:\n  holding:\n    - address: 0\n      count: 1\n      extra: true\nfaults: []\n",
        ),
        (
            "coil block",
            "device: {}\nregisters:\n  coils:\n    - address: 0\n      count: 1\n      extra: true\nfaults: []\n",
        ),
        (
            "fault",
            "device: {}\nregisters: {}\nfaults:\n  - type: timeout\n    extra: true\n",
        ),
        (
            "fault trigger",
            "device: {}\nregisters: {}\nfaults:\n  - type: timeout\n    trigger:\n      extra: true\n",
        ),
    ];

    for (name, yaml) in cases {
        let error = ModbusSimulator::from_yaml(yaml).unwrap_err();
        assert!(
            matches!(error, SimError::ConfigParse(ref message) if message.to_string().contains("unknown field")),
            "{name} unexpectedly produced {error}"
        );
    }
}

#[test]
fn from_yaml_rejects_duplicate_fields() {
    let error = ModbusSimulator::from_yaml(
        "device:\n  unit_id: 1\n  unit_id: 2\nregisters: {}\nfaults: []\n",
    )
    .unwrap_err();

    assert!(
        matches!(error, SimError::ConfigParse(message) if message.to_string().contains("duplicate field"))
    );
}

#[test]
fn from_config_and_yaml_reject_unsupported_update_modes() {
    for mode in [UpdateMode::Random, UpdateMode::Increment] {
        let mut config = generic_io();
        config.registers.holding[0].mode = mode;
        let error = config_error(config);
        assert!(error.contains("only static is supported"), "{error}");
    }

    let error = ModbusSimulator::from_yaml(
        "device: {}\nregisters:\n  holding:\n    - address: 0\n      count: 1\n      mode: random\nfaults: []\n",
    )
    .unwrap_err();
    assert!(
        matches!(error, SimError::Config(message) if message.contains("only static is supported"))
    );
}

#[test]
fn from_config_and_yaml_reject_faults() {
    let mut config = generic_io();
    config.faults.push(FaultConfig {
        fault_type: FaultType::Timeout,
        trigger: FaultTrigger::default(),
        exception: None,
        delay_ms: None,
        probability: None,
    });
    assert_eq!(
        config_error(config),
        "faults are unsupported; remove all fault entries"
    );

    let error =
        ModbusSimulator::from_yaml("device: {}\nregisters: {}\nfaults:\n  - type: timeout\n")
            .unwrap_err();
    assert!(
        matches!(error, SimError::Config(message) if message == "faults are unsupported; remove all fault entries")
    );
}

#[test]
fn direct_tcp_unit_ids_are_accepted_and_other_ids_are_rejected() {
    for unit_id in [1, 247, 255] {
        let mut config = generic_io();
        config.device.unit_id = unit_id;
        ModbusSimulator::from_config(config).unwrap();
    }

    for unit_id in [0, 248, 249, 250, 251, 252, 253, 254] {
        let mut config = generic_io();
        config.device.unit_id = unit_id;
        assert_eq!(
            config_error(config),
            format!("device.unit_id must be 1..=247 or 255, got {unit_id}")
        );
    }
}

#[test]
fn from_config_and_yaml_reject_invalid_listen_addresses() {
    let mut config = generic_io();
    config.device.listen_addr = String::from("not-a-socket-address");
    assert!(config_error(config).starts_with("invalid device.listen_addr"));

    let error = ModbusSimulator::from_yaml(
        "device:\n  listen_addr: not-a-socket-address\nregisters: {}\nfaults: []\n",
    )
    .unwrap_err();
    assert!(
        matches!(error, SimError::Config(message) if message.starts_with("invalid device.listen_addr"))
    );
}

#[test]
fn every_table_rejects_zero_count() {
    let mut cases = Vec::new();

    let mut holding = generic_io();
    holding.registers.holding = vec![register_block(0, 0, vec![])];
    cases.push(("registers.holding[0].count", holding));

    let mut input = generic_io();
    input.registers.input = vec![register_block(0, 0, vec![])];
    cases.push(("registers.input[0].count", input));

    let mut coils = generic_io();
    coils.registers.coils = vec![coil_block(0, 0, vec![])];
    cases.push(("registers.coils[0].count", coils));

    let mut discrete = generic_io();
    discrete.registers.discrete_inputs = vec![coil_block(0, 0, vec![])];
    cases.push(("registers.discrete_inputs[0].count", discrete));

    for (path, config) in cases {
        assert_eq!(config_error(config), format!("{path} must be nonzero"));
    }
}

#[test]
fn every_table_rejects_address_overflow() {
    let mut cases = Vec::new();

    let mut holding = generic_io();
    holding.registers.holding = vec![register_block(u16::MAX, 2, vec![])];
    cases.push(("registers.holding[0]", holding));

    let mut input = generic_io();
    input.registers.input = vec![register_block(u16::MAX, 2, vec![])];
    cases.push(("registers.input[0]", input));

    let mut coils = generic_io();
    coils.registers.coils = vec![coil_block(u16::MAX, 2, vec![])];
    cases.push(("registers.coils[0]", coils));

    let mut discrete = generic_io();
    discrete.registers.discrete_inputs = vec![coil_block(u16::MAX, 2, vec![])];
    cases.push(("registers.discrete_inputs[0]", discrete));

    for (path, config) in cases {
        let error = config_error(config);
        assert!(
            error.starts_with(path) && error.contains("exceeds Modbus address space"),
            "{error}"
        );
    }
}

#[test]
fn register_and_coil_blocks_reject_overlong_initial_values() {
    let mut registers = generic_io();
    registers.registers.holding = vec![register_block(0, 1, vec![1, 2])];
    assert_eq!(
        config_error(registers),
        "registers.holding[0].initial has 2 values but count is 1"
    );

    let mut input = generic_io();
    input.registers.input = vec![register_block(0, 1, vec![1, 2])];
    assert_eq!(
        config_error(input),
        "registers.input[0].initial has 2 values but count is 1"
    );

    let mut coils = generic_io();
    coils.registers.coils = vec![coil_block(0, 1, vec![true, false])];
    assert_eq!(
        config_error(coils),
        "registers.coils[0].initial has 2 values but count is 1"
    );

    let mut discrete = generic_io();
    discrete.registers.discrete_inputs = vec![coil_block(0, 1, vec![true, false])];
    assert_eq!(
        config_error(discrete),
        "registers.discrete_inputs[0].initial has 2 values but count is 1"
    );
}

#[test]
fn static_register_blocks_require_canonical_bounds() {
    for (min, max) in [(1, u16::MAX), (0, 1000), (1, 1000)] {
        let mut config = generic_io();
        config.registers.holding[0].min = min;
        config.registers.holding[0].max = max;
        assert_eq!(
            config_error(config),
            format!("registers.holding[0] static min/max must be 0/65535, got {min}/{max}")
        );
    }
}

#[test]
fn same_table_overlaps_are_rejected_but_adjacent_blocks_are_accepted() {
    let mut holding = generic_io();
    holding.registers.holding = vec![register_block(10, 2, vec![]), register_block(11, 2, vec![])];
    assert!(config_error(holding).contains(
        "registers.holding[0] range 10..=11 overlaps registers.holding[1] range 11..=12"
    ));

    let mut input = generic_io();
    input.registers.input = vec![register_block(10, 2, vec![]), register_block(11, 2, vec![])];
    assert!(
        config_error(input)
            .contains("registers.input[0] range 10..=11 overlaps registers.input[1] range 11..=12")
    );

    let mut coils = generic_io();
    coils.registers.coils = vec![coil_block(20, 2, vec![]), coil_block(21, 2, vec![])];
    assert!(
        config_error(coils)
            .contains("registers.coils[0] range 20..=21 overlaps registers.coils[1] range 21..=22")
    );

    let mut discrete = generic_io();
    discrete.registers.discrete_inputs = vec![coil_block(20, 2, vec![]), coil_block(21, 2, vec![])];
    assert!(config_error(discrete).contains("registers.discrete_inputs[0] range 20..=21 overlaps registers.discrete_inputs[1] range 21..=22"));

    let mut adjacent = generic_io();
    adjacent.registers.holding = vec![register_block(10, 2, vec![]), register_block(12, 2, vec![])];
    adjacent.registers.coils = vec![coil_block(20, 2, vec![]), coil_block(22, 2, vec![])];
    adjacent.registers.input = vec![register_block(30, 2, vec![]), register_block(32, 2, vec![])];
    adjacent.registers.discrete_inputs = vec![coil_block(40, 2, vec![]), coil_block(42, 2, vec![])];
    ModbusSimulator::from_config(adjacent).unwrap();
}

#[test]
fn identical_ranges_in_different_tables_are_accepted() {
    let mut config = generic_io();
    config.registers.holding = vec![register_block(100, 4, vec![])];
    config.registers.input = vec![register_block(100, 4, vec![])];
    config.registers.coils = vec![coil_block(100, 4, vec![])];
    config.registers.discrete_inputs = vec![coil_block(100, 4, vec![])];

    ModbusSimulator::from_config(config).unwrap();
}

#[test]
fn every_address_can_be_covered_by_adjacent_single_register_blocks() {
    let mut config = generic_io();
    config.registers.holding = (0..=u16::MAX)
        .map(|address| register_block(address, 1, vec![]))
        .collect();

    ModbusSimulator::from_config(config).unwrap();
}
