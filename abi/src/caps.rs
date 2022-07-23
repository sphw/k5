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

#[derive(Clone, defmt::Format)]
#[repr(C)]

pub struct Listen {
    pub port: [u8; 10],
}
#[derive(Clone, defmt::Format)]
#[repr(C)]
pub struct Connect {
    pub port: [u8; 10],
}
