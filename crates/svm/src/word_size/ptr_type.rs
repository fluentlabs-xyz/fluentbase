#[derive(Clone, Copy, Debug)]
pub enum PtrType {
    RcStartPtr(u64),
    RcBoxStartPtr(u64),
    PtrToValuePtr(u64),
}

impl PtrType {
    pub fn as_ref(&self) -> &u64 {
        match self {
            PtrType::RcStartPtr(v) => v,
            PtrType::RcBoxStartPtr(v) => v,
            PtrType::PtrToValuePtr(v) => v,
        }
    }
    pub fn as_ref_mut(&mut self) -> &mut u64 {
        match self {
            PtrType::RcStartPtr(v) => v,
            PtrType::RcBoxStartPtr(v) => v,
            PtrType::PtrToValuePtr(v) => v,
        }
    }
    pub fn visit_inner<F: Fn(&u64)>(&mut self, f: F) {
        f(self.as_ref());
    }
    pub fn visit_inner_mut<F: Fn(&mut u64)>(&mut self, f: F) {
        f(self.as_ref_mut());
    }
    pub fn visit_mut<F: Fn(&mut Self)>(&mut self, f: F) {
        f(self);
    }
}

#[cfg(test)]
mod tests {
    use crate::word_size::ptr_type::PtrType;

    #[test]
    fn ptr_type_modify_inner_test() {
        let inner_val1 = 12;
        let inner_val2 = 13;
        let mut ptr1 = PtrType::PtrToValuePtr(inner_val1);
        ptr1.visit_inner_mut(|v| *v = inner_val2);
        assert_eq!(ptr1.as_ref(), &inner_val2);
        assert!(matches!(ptr1, PtrType::PtrToValuePtr(_)));

        let ptr2 = PtrType::RcBoxStartPtr(inner_val1);
        ptr1.visit_mut(|v| *v = ptr2);
        assert!(matches!(ptr1, PtrType::RcBoxStartPtr(_)));
    }
}
