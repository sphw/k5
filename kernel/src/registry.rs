use abi::{Connect, Endpoint, Listen, PortId};

#[derive(Default)]
pub(crate) struct Registry {
    index: heapless::FnvIndexMap<PortId, Endpoint, 8>,
}

impl Registry {
    pub(crate) fn listen(&mut self, listen: Listen, endpoint: Endpoint) -> Result<(), abi::Error> {
        self.index
            .insert(listen.port, endpoint)
            .map_err(|_| abi::Error::BufferOverflow)?;
        Ok(())
    }

    #[allow(dead_code)]
    pub(crate) fn close(&mut self, port: PortId) -> Result<(), abi::Error> {
        self.index.remove(&port).ok_or(abi::Error::BufferOverflow)?;
        Ok(())
    }

    pub(crate) fn connect(&mut self, connect: Connect) -> Result<Endpoint, abi::Error> {
        self.index
            .get(&connect.port)
            .ok_or(abi::Error::PortNotOpen)
            .cloned()
    }
}
