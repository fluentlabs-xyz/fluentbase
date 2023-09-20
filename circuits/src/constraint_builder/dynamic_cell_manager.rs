use crate::{
    constraint_builder::{AdviceColumn, SelectorColumn},
    util::Field,
};
use halo2_proofs::plonk::ConstraintSystem;
use log::debug;
use std::marker::PhantomData;

#[derive(Clone, Debug)]
pub enum CellType {
    Selector,
    Advice,
}

#[derive(Clone, Debug)]
pub(crate) struct Cell<F> {
    cell_type: CellType,
    selector_column: Option<SelectorColumn>,
    advice_column: Option<AdviceColumn>,
    _marker: PhantomData<F>,
}

impl<F: Field> Cell<F> {
    pub(crate) fn new(
        selector_column: Option<SelectorColumn>,
        advice_column: Option<AdviceColumn>,
    ) -> Self {
        if selector_column.is_some() ^ advice_column.is_some() == false {
            panic!("exactly one column must be set")
        }
        let cell_type = if selector_column.is_some() {
            CellType::Selector
        } else {
            CellType::Advice
        };
        Self {
            cell_type,
            selector_column,
            advice_column,
            _marker: Default::default(),
        }
    }
}

impl<F: Field> TryInto<AdviceColumn> for Cell<F> {
    type Error = ();

    fn try_into(self) -> Result<AdviceColumn, Self::Error> {
        match self.cell_type {
            CellType::Advice => Ok(self.advice_column.unwrap()),
            _ => Err(()),
        }
    }
}

impl<F: Field> TryInto<SelectorColumn> for Cell<F> {
    type Error = ();

    fn try_into(self) -> Result<SelectorColumn, Self::Error> {
        match self.cell_type {
            CellType::Selector => Ok(self.selector_column.unwrap()),
            _ => Err(()),
        }
    }
}

#[derive(Clone, Debug)]
pub struct DynamicCellManager<F> {
    max_width: usize,
    height_idx: usize,
    selector_cells: Vec<Cell<F>>,
    selector_cell_idx: usize,
    advice_cells: Vec<Cell<F>>,
    advice_cell_idx: usize,
}

impl<F: Field> DynamicCellManager<F> {
    pub(crate) fn new(max_width: usize) -> Self {
        Self {
            max_width,
            height_idx: 0,
            selector_cells: vec![],
            selector_cell_idx: 0,
            advice_cells: vec![],
            advice_cell_idx: 0,
        }
    }

    fn get_allocated_width(&self) -> usize {
        self.selector_cells.len() + self.advice_cells.len()
    }

    fn get_not_allocated_width(&self) -> usize {
        self.max_width - self.get_allocated_width()
    }

    fn can_query_without_allocation_count(&self, cell_type: &CellType) -> usize {
        match cell_type {
            CellType::Selector => self.selector_cells.len() - self.selector_cell_idx,
            CellType::Advice => self.advice_cells.len() - self.advice_cell_idx,
        }
    }

    fn can_query_count(&self, cell_type: &CellType) -> usize {
        self.can_query_without_allocation_count(cell_type) + self.get_not_allocated_width()
    }

    pub fn get_height_idx(&self) -> usize {
        self.height_idx
    }

    fn allocate_more_cells(
        &mut self,
        cs: &mut ConstraintSystem<F>,
        cell_type: &CellType,
        count: usize,
    ) {
        let not_allocated_width = self.get_not_allocated_width();
        if count > self.get_not_allocated_width() {
            panic!(
                "failed to allocate {} only {} left",
                count, not_allocated_width
            );
        }
        match cell_type {
            CellType::Selector => {
                (0..count).for_each(|_| {
                    self.selector_cells
                        .push(Cell::new(Some(SelectorColumn(cs.fixed_column())), None))
                });
            }
            CellType::Advice => {
                (0..count).for_each(|_| {
                    self.advice_cells
                        .push(Cell::new(None, Some(AdviceColumn(cs.advice_column()))))
                });
            }
        }
    }

    fn get_next_cell(&mut self, cell_type: &CellType) -> Cell<F> {
        match cell_type {
            CellType::Selector => {
                let cell = self.selector_cells[self.selector_cell_idx].clone();
                self.selector_cell_idx += 1;
                cell
            }
            CellType::Advice => {
                let cell = self.advice_cells[self.advice_cell_idx].clone();
                self.advice_cell_idx += 1;
                cell
            }
        }
    }

    pub(crate) fn query_cells<const COUNT: usize>(
        &mut self,
        cs: &mut ConstraintSystem<F>,
        cell_type: &CellType,
    ) -> [Cell<F>; COUNT] {
        let can_query_count = self.can_query_count(cell_type);
        if COUNT > can_query_count {
            panic!(
                "tried to query {} of {:?} while left {} and max",
                COUNT, cell_type, can_query_count
            )
        }
        let can_query_without_allocation_count = self.can_query_without_allocation_count(cell_type);
        if COUNT > can_query_without_allocation_count {
            self.allocate_more_cells(cs, cell_type, COUNT - can_query_without_allocation_count);
        }
        let mut cells = Vec::with_capacity(COUNT);
        while cells.len() < COUNT {
            cells.push(self.get_next_cell(cell_type))
        }
        cells.try_into().unwrap()
    }

    pub fn next_line(&mut self) -> usize {
        self.height_idx += 1;
        self.selector_cell_idx = 0;
        self.advice_cell_idx = 0;
        debug!(
            "allocated_width {} (selectors {} advices {}) height_idx {} selector_cell_idx {} advice_cell_idx {}",
            self.get_allocated_width(),
            self.selector_cells.len(),
            self.advice_cells.len(),
            self.height_idx,
            self.selector_cell_idx,
            self.advice_cell_idx
        );
        self.height_idx
    }
}
