use ethereum_types::Bloom as LogBloom;

pub trait WithBloom {
    fn with_bloom(self, bloom: LogBloom) -> Self
    where
        Self: Sized;
}

pub struct Bloom<'a, I>
where
    I: 'a,
{
    pub iter: &'a mut I,
    pub bloom: LogBloom,
}

impl<'a, I> Iterator for Bloom<'a, I>
where
    I: Iterator,
    <I as Iterator>::Item: WithBloom,
{
    type Item = <I as Iterator>::Item;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.iter
            .next()
            .map(|item| item.with_bloom(self.bloom.clone()))
    }
}
