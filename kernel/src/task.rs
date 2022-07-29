use crate::regions::{Region, RegionTable};
use crate::space::Space;
use crate::task_ptr::{TaskPtr, TaskPtrMut};
use crate::{arch, KernelError};
use core::ops::Range;
use heapless::Vec;

#[repr(C)]
#[derive(Clone)]
pub(crate) struct Task {
    pub(crate) region_table: RegionTable,
    pub(crate) stack_size: usize,
    pub(crate) initial_stack_ptr: Range<usize>,
    pub(crate) available_stack_ptr: Vec<Range<usize>, 8>,
    pub(crate) entrypoint: TaskPtr<'static, fn() -> !>,
    pub(crate) secure: bool,
    pub(crate) state: TaskState,
    pub(crate) loans: Space<Loan, 16>,
}

#[repr(u8)]
#[derive(Clone, PartialEq)]
pub(crate) enum TaskState {
    Pending,
    Started,
}

impl Task {
    pub(crate) fn new(
        region_table: RegionTable,
        stack_size: usize,
        initial_stack_ptr: Range<usize>,
        entrypoint: TaskPtr<'static, fn() -> !>,
        secure: bool,
    ) -> Self {
        Self {
            region_table,
            stack_size,
            available_stack_ptr: Vec::from_slice(&[initial_stack_ptr.clone()]).unwrap(),
            initial_stack_ptr,
            secure,
            entrypoint,
            state: TaskState::Pending,
            loans: Space::default(),
        }
    }

    pub(crate) fn reset_stack_ptr(&mut self) {
        self.available_stack_ptr = Vec::from_slice(&[self.initial_stack_ptr.clone()]).unwrap()
    }

    pub(crate) fn validate_ptr<'a, T: core::ptr::Pointee + ?Sized>(
        &self,
        ptr: TaskPtr<'a, T>,
    ) -> Option<&'a T> {
        arch::translate_task_ptr(ptr, self)
    }

    pub(crate) fn validate_mut_ptr<'a, T: core::ptr::Pointee + ?Sized>(
        &self,
        ptr: TaskPtrMut<'a, T>,
    ) -> Option<&'a mut T> {
        arch::translate_mut_task_ptr(ptr, self)
    }

    pub(crate) fn alloc_stack(&mut self) -> Option<usize> {
        for range in &mut self.available_stack_ptr {
            if range.len() >= self.stack_size {
                range.start += self.stack_size;
                return Some(range.start);
                //TODO: cleanup empty ranges might need to use LL
            }
        }
        None
    }

    #[allow(dead_code)]
    pub(crate) fn make_stack_available(&mut self, stack_start: usize) {
        for range in &mut self.available_stack_ptr {
            if range.start == stack_start + self.stack_size {
                range.start = stack_start;
                return;
            }
            if range.end == stack_start {
                range.end = stack_start + self.stack_size;
                return;
            }
        }
        let _ = self
            .available_stack_ptr
            .push(stack_start..stack_start + self.stack_size);
    }

    pub(crate) fn push_loan(&mut self, loan: Loan) -> Result<LoanRef, KernelError> {
        let i = self
            .loans
            .push(loan.clone())
            .ok_or(KernelError::ABI(abi::Error::BufferOverflow))?;
        self.region_table.push(loan.region)?;
        Ok(LoanRef(i))
    }

    pub(crate) fn pop_loan(&mut self, loan_ref: LoanRef) -> Result<(), KernelError> {
        let loan = self
            .loans
            .remove(loan_ref.0)
            .ok_or(KernelError::ABI(abi::Error::InvalidLoan))?;
        self.region_table.pop(loan.region);
        Ok(())
    }
}

#[derive(Clone)]
pub(crate) struct Loan {
    pub region: Region,
}

pub(crate) struct LoanRef(usize);
