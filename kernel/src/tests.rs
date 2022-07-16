use super::*;

fn test_kernel() -> Kernel {
    Kernel::new(
        heapless::Vec::from_slice(&[Task::new(
            0..0x400,
            0..0x400,
            100,
            Vec::from_slice(&[0..200]).unwrap(),
            Vec::default(),
            unsafe { TaskPtr::from_raw_parts(1, ()) },
            false,
        )])
        .unwrap(),
        TaskRef(0),
    )
    .unwrap()
}

#[test]
fn test_simple_tick_schedule() {
    let mut kernel = test_kernel();
    let a = TCB::new(TaskRef(1), 0, 7, 5, 6, 0);
    let b = TCB::new(TaskRef(2), 0, 7, 3, 3, 0);
    kernel.scheduler.spawn(a).unwrap();
    kernel.scheduler.spawn(b).unwrap();
    for _ in 0..5 {
        let next = kernel
            .scheduler
            .tick()
            .unwrap()
            .expect("should switch to a");
        assert_eq!(*next, 1, "should switch to a");
        for _ in 0..4 {
            let next = kernel.scheduler.tick().unwrap();
            assert_eq!(next, None);
        }
        let next = kernel
            .scheduler
            .tick()
            .unwrap()
            .expect("should switch to b");
        assert_eq!(*next, 2);
        for _ in 0..2 {
            let next = kernel.scheduler.tick().unwrap();
            assert_eq!(next, None);
        }
        let next = kernel
            .scheduler
            .tick()
            .unwrap()
            .expect("should switch to idle");
        assert_eq!(*next, 0);
        for _ in 0..2 {
            let next = kernel.scheduler.tick().unwrap();
            assert_eq!(next, None);
        }
        let next = kernel
            .scheduler
            .tick()
            .unwrap()
            .expect("should switch to b");
        assert_eq!(*next, 2);
        for _ in 0..2 {
            let next = kernel.scheduler.tick().unwrap();
            assert_eq!(next, None);
        }
    }
}

#[test]
fn test_send_schedule() {
    let mut kernel = test_kernel();
    let a = TCB::new(TaskRef(1), 0, 7, 5, 6, 0);
    let mut b = TCB::new(TaskRef(2), 0, 7, 3, 3, 0);
    b.capabilities
        .push(Capability::Endpoint(Endpoint {
            tcb_ref: ThreadRef(1),
            addr: 1,
        }))
        .map_err(|_| ())
        .unwrap();
    kernel.scheduler.spawn(a).unwrap();
    kernel.scheduler.spawn(b).unwrap();
    let next = kernel
        .scheduler
        .tick()
        .unwrap()
        .expect("should switch to a");
    assert_eq!(*next, 1, "should switch to a");
    let next = kernel
        .wait(0x1, unsafe { TaskPtrMut::from_raw_parts(0, 0) })
        .unwrap();
    assert_eq!(*next, 2, "should switch to b");
    let msg = [1u8, 2, 3];
    kernel
        .send(CapabilityRef(0), Box::new(msg))
        .expect("send failed");
    for _ in 0..2 {
        let next = kernel.scheduler.tick().unwrap();
        assert_eq!(next, None);
    }
    let next = kernel
        .scheduler
        .tick()
        .unwrap()
        .expect("should switch to a");
    assert_eq!(*next, 1, "should switch to a");
}

#[test]
fn test_call_schedule() {
    let mut kernel = test_kernel();
    let a = TCB::new(TaskRef(1), 0, 7, 5, 6, 0);
    let mut b = TCB::new(TaskRef(2), 0, 7, 3, 3, 0);
    b.capabilities
        .push(Capability::Endpoint(Endpoint {
            tcb_ref: ThreadRef(1),
            addr: 1,
        }))
        .map_err(|_| ())
        .unwrap();
    kernel.scheduler.spawn(a).unwrap();
    kernel.scheduler.spawn(b).unwrap();
    let next = kernel
        .scheduler
        .tick()
        .unwrap()
        .expect("should switch to a");
    assert_eq!(*next, 1, "should switch to a");
    let next = kernel
        .wait(0x1, unsafe { TaskPtrMut::from_raw_parts(0, 0) })
        .unwrap();
    assert_eq!(*next, 2, "should switch to b");
    let msg = Box::new([1u8, 2, 3]);
    let next = kernel
        .call(CapabilityRef(0), msg, unsafe {
            TaskPtrMut::from_raw_parts(0, 0)
        })
        .expect("send failed");
    assert_eq!(*next, 1, "should switch to a");
}

#[test]
fn test_alloc_stack() {
    let mut task = Task::new(
        0..1,
        0..1,
        10,
        Vec::from_slice(&[0..50]).unwrap(),
        Vec::default(),
        unsafe { TaskPtr::from_raw_parts(0, ()) },
        false,
    );
    for i in 0..5 {
        assert_eq!(task.alloc_stack(), Some(i * 10));
    }
    assert_eq!(task.alloc_stack(), None);
    task.make_stack_available(0);
    assert_eq!(task.alloc_stack(), Some(0));
    task.make_stack_available(10);
    assert_eq!(task.alloc_stack(), Some(10));
    task.make_stack_available(40);
    assert_eq!(task.alloc_stack(), Some(40));
}
