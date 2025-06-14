use crate::error::RuntimeError;
use core::{
    fmt::{Display, Formatter},
    ops::Add,
};
use num_traits::ToPrimitive;

#[derive(Debug, Copy, Clone)]
pub enum AddrType {
    Vm(u64),
    Host(u64),
}

impl Default for AddrType {
    fn default() -> Self {
        Self::Vm(0)
    }
}

impl Display for AddrType {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        match self {
            AddrType::Vm(v) => write!(f, "AddrType::Vm({})", v),
            AddrType::Host(v) => write!(f, "AddrType::Host({})", v),
        }
    }
}

impl Add<u64> for AddrType {
    type Output = AddrType;

    fn add(self, rhs: u64) -> Self::Output {
        match self {
            AddrType::Vm(v) => AddrType::Vm(v + rhs),
            AddrType::Host(v) => AddrType::Host(v + rhs),
        }
    }
}

impl PartialEq for AddrType {
    fn eq(&self, other: &Self) -> bool {
        core::mem::discriminant(self) == core::mem::discriminant(other)
    }
}

impl AsRef<u64> for AddrType {
    fn as_ref(&self) -> &u64 {
        match self {
            AddrType::Vm(v) => v,
            AddrType::Host(v) => v,
        }
    }
}

impl AsMut<u64> for AddrType {
    fn as_mut(&mut self) -> &mut u64 {
        match self {
            AddrType::Vm(v) => v,
            AddrType::Host(v) => v,
        }
    }
}

impl<T: ToPrimitive> From<T> for AddrType {
    fn from(value: T) -> Self {
        Self::new_vm(value.to_u64().unwrap())
    }
}

impl AddrType {
    pub fn new_vm(v: u64) -> Self {
        Self::Vm(v)
    }
    pub fn new_host(v: u64) -> Self {
        Self::Host(v)
    }

    pub fn inner(&self) -> u64 {
        match self {
            AddrType::Vm(v) => *v,
            AddrType::Host(v) => *v,
        }
    }
    pub fn is_vm(&self) -> Result<u64, RuntimeError> {
        if !matches!(self, AddrType::Vm(_)) {
            return Err(RuntimeError::InvalidType);
        }
        Ok(self.inner())
    }
    pub fn is_host(&self) -> Result<u64, RuntimeError> {
        if !matches!(self, AddrType::Host(_)) {
            return Err(RuntimeError::InvalidType);
        }
        Ok(self.inner())
    }

    pub fn visit_mut<F: FnMut(&mut Self)>(&mut self, mut f: F) {
        f(self)
    }

    pub fn try_transform_to_host<F: FnMut(u64) -> u64>(
        &mut self,
        mut f: F,
    ) -> Result<(), RuntimeError> {
        if self.is_vm().is_err() {
            return Err(RuntimeError::InvalidTransformation);
        }
        *self = AddrType::Host(f(self.inner()));
        Ok(())
    }

    pub fn try_transform_to_vm<F: FnMut(&u64) -> u64>(
        &mut self,
        mut f: F,
    ) -> Result<(), RuntimeError> {
        if self.is_host().is_err() {
            return Err(RuntimeError::InvalidTransformation);
        }
        *self = AddrType::Vm(f(self.as_ref()));
        Ok(())
    }

    pub fn visit_inner_mut<F: FnMut(&mut u64)>(&mut self, mut f: F) {
        f(self.as_mut())
    }
}
