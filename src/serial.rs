pub fn scan_ports() -> Vec<String> {
    match serialport::available_ports() {
        Ok(ports) => ports.into_iter().map(|port| port.port_name).collect(),

        Err(_) => Vec::new(),
    }
}
