use core::ptr::Pointee;

#[derive(Copy, Clone)]
#[repr(transparent)]
pub struct TaskPtr<'a, T: ?Sized> {
    ptr: &'a T,
}

impl<'a, T: Pointee + ?Sized> TaskPtr<'a, T> {
    pub unsafe fn from_raw_parts(addr: usize, metadata: T::Metadata) -> Self {
        TaskPtr {
            ptr: &*core::ptr::from_raw_parts(addr as *const (), metadata),
        }
    }

    pub(crate) unsafe fn ptr(&self) -> &'a T {
        self.ptr
    }

    #[inline]
    pub(crate) fn addr(&self) -> usize {
        let (ptr, _) = (self.ptr as *const T).to_raw_parts();
        ptr.addr()
    }
}

#[derive(Debug)]
#[repr(transparent)]
pub struct TaskPtrMut<'a, T: ?Sized> {
    ptr: &'a mut T,
}

impl<'a, T: Pointee + ?Sized> TaskPtrMut<'a, T> {
    pub unsafe fn from_raw_parts(addr: usize, metadata: T::Metadata) -> Self {
        TaskPtrMut {
            ptr: &mut *core::ptr::from_raw_parts_mut(addr as *mut (), metadata),
        }
    }

    pub(crate) unsafe fn ptr(self) -> &'a mut T {
        self.ptr
    }
}

// #[cfg(test)]
// mod tests {
//     use core::mem::size_of;

//     use super::TaskPtr;

//     struct Foo {
//         a: usize,
//         b: usize,
//     }
//     #[test]
//     fn standard_ptr_validate() {
//         let foo = Foo { a: 20, b: 30 };
//         let addr = (&foo as *const Foo).addr();
//         let ptr: TaskPtr<Foo> = unsafe { TaskPtr::from_raw_parts(addr, ()) };
//         unsafe {
//             ptr.validate(&(addr..addr + size_of::<Foo>() + 1))
//                 .expect("validation failed");
//         }
//     }

//     #[test]
//     fn slice_ptr_validate() {
//         let foo = &[10u32; 20];
//         let addr = foo.as_ptr().addr();
//         let ptr: TaskPtr<[u32]> = unsafe { TaskPtr::from_raw_parts(addr, 20) };
//         unsafe {
//             ptr.validate(&(addr..addr + 4 * 20 + 1))
//                 .expect("validation failed");
//         }
//     }
// }
