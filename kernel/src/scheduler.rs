use super::KernelError;

use crate::linked_impl;
use crate::space::Space;
use crate::task_ptr::TaskPtrMut;
use crate::tcb::Tcb;
use crate::{DomainEntry, ThreadState, PRIORITY_COUNT, TCB_CAPACITY};
use abi::{RecvResp, ThreadRef};
use alloc::boxed::Box;
use cordyceps::{list::Links, List};
use core::mem::MaybeUninit;
use core::pin::Pin;
use defmt::Format;

pub(crate) struct Scheduler {
    pub(crate) tcbs: Space<Tcb, TCB_CAPACITY>,
    pub(crate) domains: [List<DomainEntry>; PRIORITY_COUNT],
    pub(crate) exhausted_threads: List<ExhaustedThread>,
    pub(crate) current_thread: ThreadTime,
}

impl Scheduler {
    pub fn spawn(&mut self, tcb: Tcb) -> Result<ThreadRef, KernelError> {
        let d = self
            .domains
            .get_mut(tcb.priority)
            .ok_or(KernelError::InvalidPriority)?;
        let tcb_ref = ThreadRef(self.tcbs.push(tcb).ok_or(KernelError::TooManyThreads)?);
        d.push_back(Box::pin(DomainEntry {
            tcb_ref,
            _links: Default::default(),
            loaned_tcb: None,
        }));
        Ok(tcb_ref)
    }

    pub(crate) fn wait(
        &mut self,
        mask: usize,
        out_buf: TaskPtrMut<'static, [u8]>,
        recv_resp: TaskPtrMut<'static, MaybeUninit<RecvResp>>,
        loan: bool,
    ) -> Result<ThreadRef, KernelError> {
        let src = self.current_thread_mut()?;
        src.state = ThreadState::Waiting {
            addr: mask,
            out_buf,
            recv_resp,
        };

        let mut next_thread = self.next_thread(0).unwrap_or_else(DomainEntry::idle);
        if loan {
            next_thread.loaned_tcb = Some(self.current_thread.tcb_ref)
        }
        self.switch_thread(next_thread)
    }

    pub fn next_thread(&mut self, current_priority: usize) -> Option<DomainEntry> {
        for domain in self
            .domains
            .iter_mut()
            .rev()
            .take(PRIORITY_COUNT - 1 - current_priority)
        {
            if let Some(thread) = domain.pop_front() {
                return Some(DomainEntry {
                    _links: Default::default(),
                    tcb_ref: thread.tcb_ref,
                    loaned_tcb: thread.loaned_tcb,
                });
            }
        }
        None
    }

    pub fn add_thread(&mut self, priority: usize, tcb_ref: ThreadRef) -> Result<(), KernelError> {
        let d = self
            .domains
            .get_mut(priority)
            .ok_or(KernelError::InvalidPriority)?;
        d.push_back(Box::pin(DomainEntry::new(tcb_ref, None)));
        Ok(())
    }

    pub fn tick(&mut self) -> Result<Option<ThreadRef>, KernelError> {
        defmt::trace!("tick");
        // requeue exhausted threads
        {
            let mut cursor = self.exhausted_threads.cursor_front_mut();
            cursor.move_prev(); // THIS IS PROBABLY WRONG
            let mut remove_flag = false;
            while let Some(t) = {
                if remove_flag {
                    cursor.remove_current();
                }
                cursor.move_next();
                cursor.current_mut()
            } {
                if let Some(tcb_ref) = t.tcb_ref {
                    let loaned_tcb = t.loaned_tcb;
                    let time = t.decrement();
                    defmt::trace!("decrement exhausted thread: {:?} {:?}", tcb_ref, time);
                    if time == 0 {
                        remove_flag = true;
                        let tcb = self
                            .tcbs
                            .get(*tcb_ref)
                            .ok_or(KernelError::InvalidThreadRef)?;
                        let d = self
                            .domains
                            .get_mut(tcb.priority)
                            .ok_or(KernelError::InvalidPriority)?;
                        d.push_back(Box::pin(DomainEntry::new(tcb_ref, loaned_tcb)));
                    }
                }
            }
        }
        self.current_thread.time -= 1;
        defmt::trace!("current thread time: {:?}", self.current_thread.time);
        // check if current thread's budget has been surpassed
        if self.current_thread.time == 0 {
            let current_tcb = self
                .tcbs
                .get(*self.current_thread.time_thread())
                .ok_or(KernelError::InvalidThreadRef)?;
            let exhausted_thread = ExhaustedThread {
                tcb_ref: Some(self.current_thread.tcb_ref),
                time: current_tcb.cooldown,
                loaned_tcb: self.current_thread.loaned_tcb,
                _links: Default::default(),
            };
            self.exhausted_threads
                .push_front(Box::pin(exhausted_thread));
            defmt::trace!("exhausting: {:?}", self.current_thread);
            let next_thread = self.next_thread(0).unwrap_or_else(DomainEntry::idle);
            return self.switch_thread(next_thread).map(Some);
        }
        let current_tcb = self.current_thread()?;
        let current_priority = current_tcb.priority;
        if let Some(next_thread) = self.next_thread(current_priority) {
            return self.switch_thread(next_thread).map(Some);
        }
        Ok(None)
    }

    pub(crate) fn switch_thread(
        &mut self,
        next_thread: DomainEntry,
    ) -> Result<ThreadRef, KernelError> {
        let loaned_tcb = if let Some(loaned) = self.current_thread.loaned_tcb {
            self.tcbs
                .get_mut(*loaned)
                .ok_or(KernelError::InvalidThreadRef)?
        } else {
            self.tcbs
                .get_mut(*self.current_thread.tcb_ref)
                .ok_or(KernelError::InvalidThreadRef)?
        };
        loaned_tcb.rem_time = self.current_thread.time;
        // NOTE: we might want to just monomorphize this out, rather than
        // using an if statement
        let time_tcb = self
            .tcbs
            .get(*next_thread.loaned_tcb.unwrap_or(next_thread.tcb_ref))
            .ok_or(KernelError::InvalidThreadRef)?;
        defmt::trace!(
            "switching: {:?} -> {:?}",
            self.current_thread.tcb_ref,
            next_thread.tcb_ref,
        );
        self.current_thread = ThreadTime {
            tcb_ref: next_thread.tcb_ref,
            time: if time_tcb.rem_time > 0 {
                time_tcb.rem_time
            } else {
                time_tcb.budget
            },
            loaned_tcb: next_thread.loaned_tcb,
        };
        Ok(next_thread.tcb_ref)
    }

    #[inline]
    pub fn current_thread(&self) -> Result<&Tcb, KernelError> {
        self.get_tcb(self.current_thread.tcb_ref)
    }

    #[inline]
    pub fn current_thread_mut(&mut self) -> Result<&mut Tcb, KernelError> {
        self.get_tcb_mut(self.current_thread.tcb_ref)
    }

    #[inline]
    pub fn get_tcb(&self, tcb_ref: ThreadRef) -> Result<&Tcb, KernelError> {
        self.tcbs.get(*tcb_ref).ok_or(KernelError::InvalidThreadRef)
    }

    #[inline]
    pub fn get_tcb_mut(&mut self, tcb_ref: ThreadRef) -> Result<&mut Tcb, KernelError> {
        self.tcbs
            .get_mut(*tcb_ref)
            .ok_or(KernelError::InvalidThreadRef)
    }
}

#[derive(Default)]
pub(crate) struct ExhaustedThread {
    _links: Links<ExhaustedThread>,
    time: usize,
    tcb_ref: Option<ThreadRef>,
    pub(crate) loaned_tcb: Option<ThreadRef>,
}

impl ExhaustedThread {
    fn decrement(self: Pin<&mut ExhaustedThread>) -> usize {
        // Safety: We never move the underlying memory, so this is safe
        unsafe {
            let s = self.get_unchecked_mut();
            s.time = s.time.saturating_sub(1);
            s.time
        }
    }

    fn loaned_tcb(self: &Pin<&mut ExhaustedThread>) -> Option<ThreadRef> {
        self.loaned_tcb
    }
}

linked_impl! { ExhaustedThread }

#[derive(Debug, Format)]
pub(crate) struct ThreadTime {
    pub(crate) tcb_ref: ThreadRef,
    pub(crate) time: usize,
    pub(crate) loaned_tcb: Option<ThreadRef>,
}

impl ThreadTime {
    fn time_thread(&self) -> ThreadRef {
        self.loaned_tcb.unwrap_or(self.tcb_ref)
    }
}
