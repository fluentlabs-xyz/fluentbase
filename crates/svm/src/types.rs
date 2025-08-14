use core::ops::{Add, Sub};
use num_traits::Num;

#[derive(Debug, Copy, Clone, PartialEq)]
pub enum ChangeDirection {
    NoChange,
    Increase,
    Decrease,
}
impl ChangeDirection {
    pub fn is_changed(&self) -> bool {
        self != &ChangeDirection::NoChange
    }
    pub fn is_no_change(&self) -> bool {
        self == &ChangeDirection::NoChange
    }
    pub fn is_increased(&self) -> bool {
        self == &ChangeDirection::Increase
    }
    pub fn is_decreased(&self) -> bool {
        self == &ChangeDirection::Decrease
    }
}
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct BalanceChangeDescriptor<T: Num + Copy + PartialOrd> {
    pub direction: ChangeDirection,
    pub amount: T,
}
impl<T: Num + Copy + PartialOrd> BalanceChangeDescriptor<T> {
    pub fn new_no_change() -> Self {
        Self {
            direction: ChangeDirection::NoChange,
            amount: T::zero(),
        }
    }
    pub fn new_increase(amount: T) -> Option<Self> {
        if amount <= T::zero() {
            return None;
        }
        Some(Self {
            direction: ChangeDirection::Increase,
            amount,
        })
    }
    pub fn new_decrease(amount: T) -> Option<Self> {
        if amount <= T::zero() {
            return None;
        }
        Some(Self {
            direction: ChangeDirection::Decrease,
            amount,
        })
    }
    pub fn add_amount(&mut self, amount: T) {
        match self.direction {
            ChangeDirection::NoChange => {
                if amount > T::zero() {
                    self.direction = ChangeDirection::Increase;
                    self.amount = amount;
                }
            }
            ChangeDirection::Increase => {
                self.amount = self.amount + amount;
            }
            ChangeDirection::Decrease => {
                if amount > self.amount {
                    self.direction = ChangeDirection::Increase;
                    self.amount = amount - self.amount;
                } else if amount == self.amount {
                    self.direction = ChangeDirection::NoChange;
                    self.amount = T::zero();
                } else {
                    self.amount = self.amount - amount;
                }
            }
        }
    }
    pub fn sub_amount(&mut self, amount: T) {
        match self.direction {
            ChangeDirection::NoChange => {
                if amount > T::zero() {
                    self.direction = ChangeDirection::Decrease;
                    self.amount = amount;
                }
            }
            ChangeDirection::Increase => {
                if amount > self.amount {
                    self.direction = ChangeDirection::Decrease;
                    self.amount = amount - self.amount;
                } else if amount == self.amount {
                    self.direction = ChangeDirection::NoChange;
                    self.amount = T::zero();
                } else {
                    self.amount = self.amount - amount;
                }
            }
            ChangeDirection::Decrease => {
                self.amount = self.amount + amount;
            }
        }
    }
}
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct BalanceHistorySnapshot<T: Num + Copy + PartialOrd> {
    pub before: T,
    pub after: T,
}
impl<T: Num + Copy + PartialOrd> BalanceHistorySnapshot<T> {
    pub fn new(before: T, after: T) -> Self {
        Self { before, after }
    }
    pub fn get_direction(&self) -> ChangeDirection {
        if self.after > self.before {
            ChangeDirection::Increase
        } else if self.after < self.before {
            ChangeDirection::Decrease
        } else {
            ChangeDirection::NoChange
        }
    }
    pub fn get_descriptor(&self) -> BalanceChangeDescriptor<T> {
        if self.after > self.before {
            BalanceChangeDescriptor::new_increase(self.after - self.before).unwrap()
        } else if self.after < self.before {
            BalanceChangeDescriptor::new_decrease(self.before - self.after).unwrap()
        } else {
            BalanceChangeDescriptor::new_no_change()
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::types::BalanceChangeDescriptor;

    #[test]
    fn test_balance_change_descriptor() {
        let mut bcd = BalanceChangeDescriptor::new_no_change();
        bcd.add_amount(10);
        assert_eq!(BalanceChangeDescriptor::new_increase(10).unwrap(), bcd);
        bcd.sub_amount(9);
        assert_eq!(BalanceChangeDescriptor::new_increase(1).unwrap(), bcd);
        bcd.sub_amount(2);
        assert_eq!(BalanceChangeDescriptor::new_decrease(1).unwrap(), bcd);
        bcd.add_amount(1);
        assert_eq!(BalanceChangeDescriptor::new_no_change(), bcd);
    }
}
