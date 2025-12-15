#[cfg(feature = "std")]
use std::net::TcpStream;

use ferredge_core::prelude::*;

pub mod attributes;
mod handler;

#[derive(Debug)]
pub struct HttpDriver {
    dvc: Device<attributes::HttpResourceAttributes>,
}

impl Driver for HttpDriver {
	#[cfg(feature = "std")]
    async fn execute(&self, command: Command) -> CommandResult {
        let command_id = command.id.clone();
        let rs = self.dvc.resources.get(&command.resource);
        let endpoint = match &self.dvc.endpoint {
            DeviceEndpoint::Http { url } => url,
            _ => "",
        };
        let tcp_stream = TcpStream::connect(endpoint)
            .map_err(|e| format!("Failed to connect: {}", e))
            .unwrap();
        match rs {
            Some(resource) => {
                let res = handler::send_request(
                    &tcp_stream,
                    &match &self.dvc.endpoint {
                        DeviceEndpoint::Http { url } => url,
                        _ => "",
                    },
                    &resource.resource_attributes,
                )
                .map_err(|e| e.to_string());
                CommandResult {
                    command_id,
                    device_id: self.dvc.id.clone(),
                    res: res,
                }
            }
            None => CommandResult {
                command_id,
                device_id: self.dvc.id.clone(),
                res: Err(format!("Resource {} not found", command.resource)),
            },
        }
    }

    async fn start(&self) -> Result<(), String> {
        // no initialization needed for HTTP driver
        Ok(())
    }

    async fn stop(&self) -> Result<(), String> {
        // no cleanup needed for HTTP driver
        Ok(())
    }
}
