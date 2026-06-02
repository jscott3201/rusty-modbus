use rusty_modbus_client::ClientError;
use rusty_modbus_types::UnitId;

use crate::shell_parser::{self, ShellCommand};

use super::{CommandLogEntry, DashboardApp, DashboardStatus, DashboardTarget};

impl DashboardApp {
    pub(super) async fn execute_command_line(&mut self, line: String) {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return;
        }
        self.push_history(trimmed);
        self.push_log(CommandLogEntry::info(format!("> {trimmed}")));

        match shell_parser::parse_command(trimmed) {
            Ok(command) => self.execute_shell_command(command).await,
            Err(error) => self.record_error(format!("Parse error: {error}")),
        }
    }

    async fn execute_shell_command(&mut self, command: ShellCommand) {
        match command {
            ShellCommand::ReadCoils { address, quantity } => {
                self.read_target(DashboardTarget::Coils, address, quantity)
                    .await;
            }
            ShellCommand::ReadDiscreteInputs { address, quantity } => {
                self.read_target(DashboardTarget::DiscreteInputs, address, quantity)
                    .await;
            }
            ShellCommand::ReadHoldingRegisters { address, quantity } => {
                self.read_target(DashboardTarget::HoldingRegisters, address, quantity)
                    .await;
            }
            ShellCommand::ReadInputRegisters { address, quantity } => {
                self.read_target(DashboardTarget::InputRegisters, address, quantity)
                    .await;
            }
            ShellCommand::WriteCoil { address, value } => {
                let result = self
                    .client
                    .write_single_coil(UnitId(self.view.unit_id), address, value)
                    .await;
                self.finish_write(result, DashboardTarget::Coils, address, 1)
                    .await;
            }
            ShellCommand::WriteCoils { address, values } => {
                let quantity = u16::try_from(values.len()).unwrap_or(u16::MAX);
                let result = self
                    .client
                    .write_multiple_coils(UnitId(self.view.unit_id), address, &values)
                    .await;
                self.finish_write(result, DashboardTarget::Coils, address, quantity)
                    .await;
            }
            ShellCommand::WriteRegister { address, value } => {
                let result = self
                    .client
                    .write_single_register(UnitId(self.view.unit_id), address, value)
                    .await;
                self.finish_write(result, DashboardTarget::HoldingRegisters, address, 1)
                    .await;
            }
            ShellCommand::WriteRegisters { address, values } => {
                let quantity = u16::try_from(values.len()).unwrap_or(u16::MAX);
                let result = self
                    .client
                    .write_multiple_registers(UnitId(self.view.unit_id), address, &values)
                    .await;
                self.finish_write(result, DashboardTarget::HoldingRegisters, address, quantity)
                    .await;
            }
            ShellCommand::SetUnitId(id) => {
                self.view.unit_id = id;
                self.push_log(CommandLogEntry::success(format!("Unit ID set to {id}")));
                let _ = self.refresh().await;
            }
            ShellCommand::Status => {
                self.push_log(CommandLogEntry::info(format!(
                    "Endpoint {} unit {} connected {}",
                    self.view.endpoint,
                    self.view.unit_id,
                    self.client.is_connected()
                )));
            }
            ShellCommand::Help => {
                for line in shell_parser::HELP_LINES {
                    self.push_log(CommandLogEntry::info(*line));
                }
            }
            ShellCommand::Exit => {
                self.should_quit = true;
            }
            ShellCommand::Empty => {}
        }
    }

    async fn read_target(&mut self, target: DashboardTarget, address: u16, quantity: u16) {
        self.view.target = target;
        self.view.address = address;
        self.view.quantity = quantity;
        self.view.clamp_quantity();

        match self.refresh().await {
            Ok(rows) => self.push_log(CommandLogEntry::success(format!(
                "Read {rows} {} values from {address}",
                target.short_label()
            ))),
            Err(error) => self.push_log(CommandLogEntry::error(format!("Read failed: {error}"))),
        }
    }

    async fn finish_write(
        &mut self,
        result: Result<(), ClientError>,
        target: DashboardTarget,
        address: u16,
        quantity: u16,
    ) {
        match result {
            Ok(()) => {
                self.push_log(CommandLogEntry::success(format!(
                    "Write OK: {quantity} {} values at {address}",
                    target.short_label()
                )));
                self.view.target = target;
                self.view.address = address;
                self.view.quantity = quantity.max(1);
                self.view.clamp_quantity();
                let _ = self.refresh().await;
            }
            Err(error) => self.record_error(format!("Write failed: {error}")),
        }
    }

    fn record_error(&mut self, message: String) {
        self.view.status = DashboardStatus::Error;
        self.view.message = message.clone();
        self.push_log(CommandLogEntry::error(message));
    }
}
