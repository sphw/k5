use core::mem::MaybeUninit;

#[derive(Debug)]
pub struct Space<T, const N: usize> {
    items: [Option<T>; N],
    free_list: [usize; N],
    len: usize,
}

impl<T, const N: usize> Default for Space<T, N> {
    fn default() -> Self {
        let mut items: [MaybeUninit<Option<T>>; N] = MaybeUninit::uninit_array();
        for item in &mut items {
            item.write(None);
        }
        // Safety: we just inited all of the above items, so this is safe
        let items = unsafe { MaybeUninit::array_assume_init(items) };
        let mut free_list = [0; N];
        for (i, x) in free_list.iter_mut().enumerate() {
            *x = N - (i + 1);
        }
        Self {
            items,
            free_list,
            len: 0,
        }
    }
}

impl<T, const N: usize> Space<T, N> {
    pub fn push(&mut self, item: T) -> Option<usize> {
        if self.len >= N {
            return None;
        }
        self.len += 1;
        let i = self.free_list[N - self.len];
        self.items[i] = Some(item);
        Some(i)
    }

    pub fn get(&self, i: usize) -> Option<&T> {
        self.items.get(i).and_then(|o| o.as_ref())
    }

    pub fn get_mut(&mut self, i: usize) -> Option<&mut T> {
        self.items.get_mut(i).and_then(|o| o.as_mut())
    }

    pub fn remove(&mut self, i: usize) -> Option<T> {
        let item = self.items.get_mut(i)?.take()?;
        self.free_list[N - self.len] = i;
        self.len -= 1;
        Some(item)
    }

    #[allow(dead_code)]
    pub fn iter(&self) -> impl Iterator<Item = &T> {
        self.items.iter().filter_map(|i| i.as_ref())
    }

    #[allow(dead_code)]
    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut T> {
        self.items.iter_mut().filter_map(|i| i.as_mut())
    }

    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.len
    }
}

#[cfg(test)]
mod tests {
    use super::Space;

    #[test]
    fn test_push() {
        let mut space = Space::<usize, 10>::default();
        for i in 0..10 {
            space.push(i);
        }
        println!("{:?}", space);
        for i in 0..10 {
            assert_eq!(space.get(i), Some(&i))
        }
    }

    #[test]
    fn test_remove_push() {
        let mut space = Space::<usize, 10>::default();
        for i in 0..10 {
            space.push(i);
        }
        assert_eq!(space.push(10), None);
        space.remove(5).expect("item not found");
        assert_eq!(space.push(10), Some(5));
        for i in 0..10 {
            space.remove(i).expect("item not found");
            assert_eq!(space.push(i), Some(i));
        }
    }
}

// pub struct Iter<'a, T, const N: usize> {
//     space: &'a Space<T, N>,
//     i: usize,
// }

// impl<'a, T, const N: usize> Iterator for Iter<'a, T, N> {
//     type Item = &'a T;

//     fn next(&mut self) -> Option<Self::Item> {
//         let item = self.space.get(self.i)?;
//         self.i += 1;
//         Some(item)
//     }
// }

// impl<'a, T, const N: usize> ExactSizeIterator for Iter<'a, T, N> {
//     fn len(&self) -> usize {
//         self.space.len
//     }
// }

// pub struct IterMut<'a, T, const N: usize> {
//     space: &'a mut Space<T, N>,
//     i: usize,
// }

// impl<'a, T: 'a, const N: usize> Iterator for IterMut<'a, T, N> {
//     type Item = &'a mut T;

//     fn next(&mut self) -> Option<Self::Item> {
//         let item = &mut *self.space.get_mut(self.i)?;
//         self.i += 1;
//         Some(item)
//     }
// }

// impl<'a, T, const N: usize> ExactSizeIterator for IterMut<'a, T, N> {
//     fn len(&self) -> usize {
//         self.space.len
//     }
// }
