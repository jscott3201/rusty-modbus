//! Python-visible Modbus types.

use pyo3::prelude::*;
use rusty_modbus_frame::OwnedDeviceIdentification;

/// Device identification returned by FC 0x2B (MEI 0x0E).
#[pyclass(frozen, from_py_object, module = "rusty_modbus")]
#[derive(Debug, Clone)]
pub struct DeviceIdentification {
    #[pyo3(get)]
    pub vendor_name: Option<String>,
    #[pyo3(get)]
    pub product_code: Option<String>,
    #[pyo3(get)]
    pub major_minor_revision: Option<String>,
}

#[pymethods]
impl DeviceIdentification {
    fn __repr__(&self) -> String {
        format!(
            "DeviceIdentification(vendor_name={:?}, product_code={:?}, major_minor_revision={:?})",
            self.vendor_name, self.product_code, self.major_minor_revision,
        )
    }
}

impl From<OwnedDeviceIdentification> for DeviceIdentification {
    fn from(d: OwnedDeviceIdentification) -> Self {
        Self {
            vendor_name: d.vendor_name,
            product_code: d.product_code,
            major_minor_revision: d.major_minor_revision,
        }
    }
}
