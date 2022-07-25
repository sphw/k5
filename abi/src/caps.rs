#[derive(Clone, defmt::Format)]
#[repr(C)]
pub enum Cap {
    Endpoint(Endpoint),
    Listen(Listen),
    Connect(Connect),
    Notification,
}

#[repr(C)]
#[derive(Clone, Copy, defmt::Format)]
pub struct Endpoint {
    pub tcb_ref: super::ThreadRef,
    pub addr: usize,
    pub disposable: bool,
}

pub type PortId = [u8; 16];

#[derive(Clone, Copy, defmt::Format)]
#[repr(C)]
pub struct Listen {
    pub port: PortId,
}
#[derive(Clone, Copy, defmt::Format)]
#[repr(C)]
pub struct Connect {
    pub port: PortId,
}
