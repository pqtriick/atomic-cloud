use serde::Deserialize;

use crate::exports::node::driver::bridge::Address;

#[derive(Deserialize, Clone)]
pub struct BAllocation {
    pub id: u32,
    pub ip: String,
    pub port: u16,
    pub assigned: bool,
}

#[derive(Deserialize, Clone)]
pub struct BCAllocation {
    pub id: u32,
    pub ip: String,
    pub port: u16,
    pub is_default: bool,
}

impl From<BCAllocation> for Address {
    fn from(val: BCAllocation) -> Self {
        Address {
            ip: val.ip,
            port: val.port,
        }
    }
}
