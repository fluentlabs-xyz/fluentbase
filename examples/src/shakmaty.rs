use shakmaty::{Chess, Position};

pub fn deploy() {}

pub fn main() {
    let pos = Chess::default();
    let legals = pos.legal_moves();
    assert_eq!(legals.len(), 20);
}
