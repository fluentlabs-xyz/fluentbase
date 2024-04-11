use crate::{handler::register::EvmHandler, Inspector};
use fluentbase_types::IJournaledTrie;

/// Provides access to an `Inspector` instance.
pub trait GetInspector<DB: IJournaledTrie> {
    /// Returns the associated `Inspector`.
    fn get_inspector(&mut self) -> &mut impl Inspector<DB>;
}

impl<DB: IJournaledTrie, INSP: Inspector<DB>> GetInspector<DB> for INSP {
    #[inline(always)]
    fn get_inspector(&mut self) -> &mut impl Inspector<DB> {
        self
    }
}

/// Register Inspector handles that interact with Inspector instance.
///
///
/// # Note
///
/// Inspector handle register does not override any existing handlers, and it
/// calls them before (or after) calling Inspector. This means that it is safe
/// to use this register with any other register.
///
/// A few instructions handlers are wrapped twice once for `step` and `step_end`
/// and in case of Logs and Selfdestruct wrapper is wrapped again for the
/// `log` and `selfdestruct` calls.
pub fn inspector_handle_register<'a, DB: IJournaledTrie, EXT: GetInspector<DB>>(
    _handler: &mut EvmHandler<'a, EXT, DB>,
) {
    // Every instruction inside flat table that is going to be wrapped by inspector calls.
    // let table = handler
    //     .take_instruction_table()
    //     .expect("Handler must have instruction table");
    // let mut table = match table {
    //     InstructionTables::Plain(table) => table
    //         .into_iter()
    //         .map(|i| inspector_instruction(i))
    //         .collect::<Vec<_>>(),
    //     InstructionTables::Boxed(table) => table
    //         .into_iter()
    //         .map(|i| inspector_instruction(i))
    //         .collect::<Vec<_>>(),
    // };
    //
    // // Register inspector Log instruction.
    // let mut inspect_log = |index: u8| {
    //     if let Some(i) = table.get_mut(index as usize) {
    //         let old = core::mem::replace(i, Box::new(|_, _| ()));
    //         *i = Box::new(
    //             move |interpreter: &mut Interpreter, host: &mut Evm<'a, EXT, DB>| {
    //                 let old_log_len = host.context.evm.journaled_state.logs.len();
    //                 old(interpreter, host);
    //                 // check if log was added. It is possible that revert happened
    //                 // cause of gas or stack underflow.
    //                 if host.context.evm.journaled_state.logs.len() == old_log_len + 1 {
    //                     // clone log.
    //                     // TODO decide if we should remove this and leave the comment
    //                     // that log can be found as journaled_state.
    //                     let last_log = host
    //                         .context
    //                         .evm
    //                         .journaled_state
    //                         .logs
    //                         .last()
    //                         .unwrap()
    //                         .clone();
    //                     // call Inspector
    //                     host.context
    //                         .external
    //                         .get_inspector()
    //                         .log(&mut host.context.evm, &last_log);
    //                 }
    //             },
    //         )
    //     }
    // };
    //
    // inspect_log(opcode::LOG0);
    // inspect_log(opcode::LOG1);
    // inspect_log(opcode::LOG2);
    // inspect_log(opcode::LOG3);
    // inspect_log(opcode::LOG4);
    //
    // // // register selfdestruct function.
    // if let Some(i) = table.get_mut(opcode::SELFDESTRUCT as usize) {
    //     let old = core::mem::replace(i, Box::new(|_, _| ()));
    //     *i = Box::new(
    //         move |interpreter: &mut Interpreter, host: &mut Evm<'a, EXT, DB>| {
    //             // execute selfdestruct
    //             old(interpreter, host);
    //             // check if selfdestruct was successful and if journal entry is made.
    //             if let Some(JournalEntry::AccountDestroyed {
    //                 address,
    //                 target,
    //                 had_balance,
    //                 ..
    //             }) = host
    //                 .context
    //                 .evm
    //                 .journaled_state
    //                 .journal
    //                 .last()
    //                 .unwrap()
    //                 .last()
    //             {
    //                 host.context.external.get_inspector().selfdestruct(
    //                     *address,
    //                     *target,
    //                     *had_balance,
    //                 );
    //             }
    //         },
    //     )
    // }
    //
    // // cast vector to array.
    // handler.set_instruction_table(InstructionTables::Boxed(
    //     table.try_into().unwrap_or_else(|_| unreachable!()),
    // ));
    //
    // // call and create input stack shared between handlers. They are used to share
    // // inputs in *_end Inspector calls.
    // let call_input_stack = Rc::<RefCell<Vec<_>>>::new(RefCell::new(Vec::new()));
    // let create_input_stack = Rc::<RefCell<Vec<_>>>::new(RefCell::new(Vec::new()));
    //
    // // Create handle
    // let create_input_stack_inner = create_input_stack.clone();
    // let old_handle = handler.execution.create.clone();
    // handler.execution.create = Arc::new(
    //     move |ctx, mut inputs| -> Result<FrameOrResult, EVMError<ExitCode>> {
    //         let inspector = ctx.external.get_inspector();
    //         // call inspector create to change input or return outcome.
    //         if let Some(outcome) = inspector.create(&mut ctx.evm, &mut inputs) {
    //             create_input_stack_inner.borrow_mut().push(inputs.clone());
    //             return Ok(FrameOrResult::Result(FrameResult::Create(outcome)));
    //         }
    //         create_input_stack_inner.borrow_mut().push(inputs.clone());
    //
    //         let mut frame_or_result = old_handle(ctx, inputs);
    //         if let Ok(FrameOrResult::Frame(frame)) = &mut frame_or_result {
    //             ctx.external
    //                 .get_inspector()
    //                 .initialize_interp(frame.interpreter_mut(), &mut ctx.evm)
    //         }
    //         frame_or_result
    //     },
    // );
    //
    // // Call handler
    // let call_input_stack_inner = call_input_stack.clone();
    // let old_handle = handler.execution.call.clone();
    // handler.execution.call = Arc::new(
    //     move |ctx, mut inputs| -> Result<FrameOrResult, EVMError<ExitCode>> {
    //         // Call inspector to change input or return outcome.
    //         let outcome = ctx.external.get_inspector().call(&mut ctx.evm, &mut inputs);
    //         call_input_stack_inner.borrow_mut().push(inputs.clone());
    //         if let Some(outcome) = outcome {
    //             return Ok(FrameOrResult::Result(FrameResult::Call(outcome)));
    //         }
    //
    //         let mut frame_or_result = old_handle(ctx, inputs);
    //         if let Ok(FrameOrResult::Frame(frame)) = &mut frame_or_result {
    //             ctx.external
    //                 .get_inspector()
    //                 .initialize_interp(frame.interpreter_mut(), &mut ctx.evm)
    //         }
    //         frame_or_result
    //     },
    // );
    //
    // // call outcome
    // let call_input_stack_inner = call_input_stack.clone();
    // let old_handle = handler.execution.insert_call_outcome.clone();
    // handler.execution.insert_call_outcome =
    //     Arc::new(move |ctx, frame, shared_memory, mut outcome| {
    //         let call_inputs = call_input_stack_inner.borrow_mut().pop().unwrap();
    //         outcome = ctx
    //             .external
    //             .get_inspector()
    //             .call_end(&mut ctx.evm, &call_inputs, outcome);
    //         old_handle(ctx, frame, shared_memory, outcome)
    //     });
    //
    // // create outcome
    // let create_input_stack_inner = create_input_stack.clone();
    // let old_handle = handler.execution.insert_create_outcome.clone();
    // handler.execution.insert_create_outcome = Arc::new(move |ctx, frame, mut outcome| {
    //     let create_inputs = create_input_stack_inner.borrow_mut().pop().unwrap();
    //     outcome = ctx
    //         .external
    //         .get_inspector()
    //         .create_end(&mut ctx.evm, &create_inputs, outcome);
    //     old_handle(ctx, frame, outcome)
    // });
    //
    // // last frame outcome
    // let old_handle = handler.execution.last_frame_return.clone();
    // handler.execution.last_frame_return = Arc::new(move |ctx, frame_result| {
    //     let inspector = ctx.external.get_inspector();
    //     match frame_result {
    //         FrameResult::Call(outcome) => {
    //             let call_inputs = call_input_stack.borrow_mut().pop().unwrap();
    //             *outcome = inspector.call_end(&mut ctx.evm, &call_inputs, outcome.clone());
    //         }
    //         FrameResult::Create(outcome) => {
    //             let create_inputs = create_input_stack.borrow_mut().pop().unwrap();
    //             *outcome = inspector.create_end(&mut ctx.evm, &create_inputs, outcome.clone());
    //         }
    //     }
    //     old_handle(ctx, frame_result)
    // });
}
