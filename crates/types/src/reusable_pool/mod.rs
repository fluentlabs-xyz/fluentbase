pub mod global;
pub mod global_types;
pub mod macroses;

use alloc::vec::Vec;
use core::marker::PhantomData;

#[derive(Clone)]
pub struct ReusablePoolConfig<ITEM, Create: Fn() -> ITEM, Reset: Fn(&mut ITEM) -> bool> {
    pub keep: usize,
    pub create: Create,
    pub reset: Reset,
    pub _phantom: PhantomData<ITEM>,
}

impl<ITEM, Create: Fn() -> ITEM, Reset: Fn(&mut ITEM) -> bool>
    ReusablePoolConfig<ITEM, Create, Reset>
{
    pub fn new(keep: usize, create_behavior: Create, reset_behavior: Reset) -> Self {
        Self {
            keep,
            create: create_behavior,
            reset: reset_behavior,
            _phantom: PhantomData::default(),
        }
    }
}

#[derive(Clone)]
pub struct ReusablePool<ITEM, Create: Fn() -> ITEM, Reset: Fn(&mut ITEM) -> bool> {
    items: Vec<ITEM>,
    config: ReusablePoolConfig<ITEM, Create, Reset>,
}

impl<ITEM, Create: Fn() -> ITEM, Reset: Fn(&mut ITEM) -> bool> ReusablePool<ITEM, Create, Reset> {
    pub fn new(config: ReusablePoolConfig<ITEM, Create, Reset>) -> Self {
        Self {
            items: Vec::with_capacity(config.keep),
            config,
        }
    }

    #[inline]
    pub fn reuse_or_new(&mut self) -> ITEM {
        match self.items.pop() {
            Some(item) => item,
            None => (self.config.create)(),
        }
    }

    #[inline]
    pub fn recycle(&mut self, mut item: ITEM) {
        if self.items.len() < self.config.keep {
            let result = (self.config.reset)(&mut item);
            if result {
                self.items.push(item);
            }
        }
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.items.len()
    }

    #[inline]
    pub fn cap(&self) -> usize {
        self.items.capacity()
    }
}
