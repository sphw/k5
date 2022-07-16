use core::{mem::size_of_val, ops::Range, ptr::Pointee};

#[derive(Copy, Clone)]
pub struct TaskPtr<'a, T: ?Sized> {
    ptr: &'a T,
}

impl<'a, T: Pointee + ?Sized> TaskPtr<'a, T> {
    pub unsafe fn from_raw_parts(addr: usize, metadata: T::Metadata) -> Self {
        TaskPtr {
            ptr: &*core::ptr::from_raw_parts(addr as *const (), metadata),
        }
    }

    pub unsafe fn validate(&self, range: &Range<usize>) -> Option<&'a T> {
        let (ptr, _) = (self.ptr as *const T).to_raw_parts();
        let addr = ptr.addr();
        let end = addr + size_of_val(self.ptr);
        (range.contains(&addr) && range.contains(&end)).then(|| self.ptr)
    }
}

#[derive(Debug)]
pub struct TaskPtrMut<'a, T: ?Sized> {
    ptr: &'a mut T,
}

impl<'a, T: Pointee + ?Sized> TaskPtrMut<'a, T> {
    pub unsafe fn from_raw_parts(addr: usize, metadata: T::Metadata) -> Self {
        TaskPtrMut {
            ptr: &mut *core::ptr::from_raw_parts_mut(addr as *mut (), metadata),
        }
    }

    pub unsafe fn validate(
        &mut self,
        ram_range: &Range<usize>,
        flash_range: &Range<usize>,
    ) -> Option<&mut T> {
        let (ptr, _) = (self.ptr as *const T).to_raw_parts();
        let addr = ptr.addr();
        let end = addr + size_of_val(self.ptr);
        if (ram_range.contains(&addr) && ram_range.contains(&end))
            || (flash_range.contains(&addr) && flash_range.contains(&end))
        {
            Some(self.ptr)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use core::mem::size_of;

    use super::TaskPtr;

    struct Foo {
        a: usize,
        b: usize,
    }
    #[test]
    fn standard_ptr_validate() {
        let foo = Foo { a: 20, b: 30 };
        let addr = (&foo as *const Foo).addr();
        let ptr: TaskPtr<Foo> = unsafe { TaskPtr::from_raw_parts(addr, ()) };
        unsafe {
            ptr.validate(&(addr..addr + size_of::<Foo>() + 1))
                .expect("validation failed");
        }
    }

    #[test]
    fn slice_ptr_validate() {
        let foo = &[10u32; 20];
        let addr = foo.as_ptr().addr();
        let ptr: TaskPtr<[u32]> = unsafe { TaskPtr::from_raw_parts(addr, 20) };
        unsafe {
            ptr.validate(&(addr..addr + 4 * 20 + 1))
                .expect("validation failed");
        }
    }
}
