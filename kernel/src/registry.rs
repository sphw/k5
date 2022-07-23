use abi::{Connect, Endpoint, Listen};

use crate::KernelError;

struct Registry {
    index: heapless::FnvIndexMap<[u8; 10], Endpoint, 6>,
}

impl Registry {
    fn listen(&mut self, listen: Listen, endpoint: Endpoint) -> Result<(), abi::Error> {
        self.index
            .insert(listen.port, endpoint)
            .map_err(|_| abi::Error::BufferOverflow)?;
        Ok(())
    }

    fn close(&mut self, port: [u8; 10]) -> Result<(), abi::Error> {
        self.index.remove(&port).ok_or(abi::Error::BufferOverflow)?;
        Ok(())
    }

    fn connect(&mut self, connect: Connect) -> Result<Endpoint, abi::Error> {
        self.index
            .get(&connect.port)
            .ok_or(abi::Error::PortNotOpen)
            .cloned()
    }
}
