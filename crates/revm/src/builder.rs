use crate::{
    db::Database,
    handler::register,
    primitives::{
        BlockEnv, CfgEnv, CfgEnvWithHandlerCfg, Env, EnvWithHandlerCfg, HandlerCfg, SpecId, TxEnv,
    },
    Context, ContextWithHandlerCfg, Evm, EvmContext, Handler, InnerEvmContext,
};
use core::marker::PhantomData;
use fluentbase_types::{EmptyJournalTrie, IJournaledTrie};
use revm_primitives::ShanghaiSpec;
use std::boxed::Box;

/// Evm Builder allows building or modifying EVM.
/// Note that some of the methods that changes underlying structures
/// will reset the registered handler to default mainnet.
pub struct EvmBuilder<'a, BuilderStage, EXT, DB: IJournaledTrie> {
    context: Context<EXT, DB>,
    /// Handler that will be used by EVM. It contains handle registers
    handler: Handler<'a, EXT, DB>,
    /// Phantom data to mark the stage of the builder.
    phantom: PhantomData<BuilderStage>,
}

impl<'a, BuilderStage> Default for EvmBuilder<'a, BuilderStage, (), EmptyJournalTrie> {
    fn default() -> Self {
        Self {
            context: Context {
                evm: EvmContext {
                    inner: InnerEvmContext {
                        env: Box::new(Default::default()),
                        db: EmptyJournalTrie {},
                        error: Ok(()),
                        depth: 0,
                        spec_id: Default::default(),
                    },
                },
                external: (),
            },
            handler: Handler::mainnet::<ShanghaiSpec>(),
            phantom: Default::default(),
        }
    }
}

/// First stage of the builder allows setting generic variables.
/// Generic variables are database and external context.
pub struct SetGenericStage;

/// Second stage of the builder allows appending handler registers.
/// Requires the database and external context to be set.
pub struct HandlerStage;

impl<'a, EXT, DB: IJournaledTrie> EvmBuilder<'a, SetGenericStage, EXT, DB> {
    /// Sets the [`Database`] that will be used by [`Evm`].
    pub fn with_db<ODB: IJournaledTrie>(
        self,
        db: ODB,
    ) -> EvmBuilder<'a, SetGenericStage, EXT, ODB> {
        EvmBuilder {
            context: Context::new(self.context.evm.with_db(db), self.context.external),
            handler: EvmBuilder::<'a, SetGenericStage, EXT, ODB>::handler(self.handler.cfg()),
            phantom: PhantomData,
        }
    }

    /// Sets the external context that will be used by [`Evm`].
    pub fn with_external_context<OEXT>(
        self,
        external: OEXT,
    ) -> EvmBuilder<'a, SetGenericStage, OEXT, DB> {
        EvmBuilder {
            context: Context::new(self.context.evm, external),
            handler: EvmBuilder::<'a, SetGenericStage, OEXT, DB>::handler(self.handler.cfg()),
            phantom: PhantomData,
        }
    }

    /// Sets Builder with [`EnvWithHandlerCfg`].
    pub fn with_env_with_handler_cfg(
        mut self,
        env_with_handler_cfg: EnvWithHandlerCfg,
    ) -> EvmBuilder<'a, HandlerStage, EXT, DB> {
        let EnvWithHandlerCfg { env, handler_cfg } = env_with_handler_cfg;
        self.context.evm.env = env;
        EvmBuilder {
            context: self.context,
            handler: EvmBuilder::<'a, HandlerStage, EXT, DB>::handler(handler_cfg),
            phantom: PhantomData,
        }
    }

    /// Sets Builder with [`ContextWithHandlerCfg`].
    pub fn with_context_with_handler_cfg<OEXT, ODB: IJournaledTrie>(
        self,
        context_with_handler_cfg: ContextWithHandlerCfg<OEXT, ODB>,
    ) -> EvmBuilder<'a, HandlerStage, OEXT, ODB> {
        EvmBuilder {
            context: context_with_handler_cfg.context,
            handler: EvmBuilder::<'a, HandlerStage, OEXT, ODB>::handler(
                context_with_handler_cfg.cfg,
            ),
            phantom: PhantomData,
        }
    }

    /// Sets Builder with [`CfgEnvWithHandlerCfg`].
    pub fn with_cfg_env_with_handler_cfg(
        mut self,
        cfg_env_and_spec_id: CfgEnvWithHandlerCfg,
    ) -> EvmBuilder<'a, HandlerStage, EXT, DB> {
        self.context.evm.env.cfg = cfg_env_and_spec_id.cfg_env;

        EvmBuilder {
            context: self.context,
            handler: EvmBuilder::<'a, HandlerStage, EXT, DB>::handler(
                cfg_env_and_spec_id.handler_cfg,
            ),
            phantom: PhantomData,
        }
    }

    /// Sets Builder with [`HandlerCfg`]
    pub fn with_handler_cfg(
        self,
        handler_cfg: HandlerCfg,
    ) -> EvmBuilder<'a, HandlerStage, EXT, DB> {
        EvmBuilder {
            context: self.context,
            handler: EvmBuilder::<'a, HandlerStage, EXT, DB>::handler(handler_cfg),
            phantom: PhantomData,
        }
    }

    /// Sets the Optimism handler with latest spec.
    ///
    /// If `optimism-default-handler` feature is enabled this is not needed.
    #[cfg(feature = "optimism")]
    pub fn optimism(mut self) -> EvmBuilder<'a, HandlerStage, EXT, DB> {
        self.handler = Handler::optimism_with_spec(self.handler.cfg.spec_id);
        EvmBuilder {
            context: self.context,
            handler: self.handler,
            phantom: PhantomData,
        }
    }

    /// Sets the mainnet handler with latest spec.
    ///
    /// Enabled only with `optimism-default-handler` feature.
    #[cfg(feature = "optimism-default-handler")]
    pub fn mainnet(mut self) -> EvmBuilder<'a, HandlerStage, EXT, DB> {
        self.handler = Handler::mainnet_with_spec(self.handler.cfg.spec_id);
        EvmBuilder {
            context: self.context,
            handler: self.handler,
            phantom: PhantomData,
        }
    }
}

impl<'a, EXT, DB: IJournaledTrie> EvmBuilder<'a, HandlerStage, EXT, DB> {
    /// Creates new builder from Evm, Evm is consumed and all field are moved to Builder.
    /// It will preserve set handler and context.
    ///
    /// Builder is in HandlerStage and both database and external are set.
    pub fn new(evm: Evm<'a, EXT, DB>) -> Self {
        Self {
            context: evm.context,
            handler: evm.handler,
            phantom: PhantomData,
        }
    }

    /// Resets the [`Handler`] and sets base mainnet handler.
    ///
    /// Enabled only with `optimism-default-handler` feature.
    #[cfg(feature = "optimism-default-handler")]
    pub fn reset_handler_with_mainnet(mut self) -> EvmBuilder<'a, HandlerStage, EXT, DB> {
        self.handler = Handler::mainnet_with_spec(self.handler.cfg.spec_id);
        EvmBuilder {
            context: self.context,
            handler: self.handler,
            phantom: PhantomData,
        }
    }

    /// Sets the [`Database`] that will be used by [`Evm`]
    /// and resets the [`Handler`] to default mainnet.
    pub fn reset_handler_with_db<ODB: IJournaledTrie>(
        self,
        db: ODB,
    ) -> EvmBuilder<'a, SetGenericStage, EXT, ODB> {
        EvmBuilder {
            context: Context::new(self.context.evm.with_db(db), self.context.external),
            handler: EvmBuilder::<'a, SetGenericStage, EXT, ODB>::handler(self.handler.cfg()),
            phantom: PhantomData,
        }
    }

    /// Resets [`Handler`] and sets new `ExternalContext` type.
    ///  and resets the [`Handler`] to default mainnet.
    pub fn reset_handler_with_external_context<OEXT>(
        self,
        external: OEXT,
    ) -> EvmBuilder<'a, SetGenericStage, OEXT, DB> {
        EvmBuilder {
            context: Context::new(self.context.evm, external),
            handler: EvmBuilder::<'a, SetGenericStage, OEXT, DB>::handler(self.handler.cfg()),
            phantom: PhantomData,
        }
    }
}

impl<'a, BuilderStage, EXT, DB: IJournaledTrie> EvmBuilder<'a, BuilderStage, EXT, DB> {
    /// Creates the default handler.
    ///
    /// This is useful for adding optimism handle register.
    fn handler(handler_cfg: HandlerCfg) -> Handler<'a, EXT, DB> {
        Handler::new(handler_cfg)
    }

    /// This modifies the [EvmBuilder] to make it easy to construct an [`Evm`] with a _specific_
    /// handler.
    ///
    /// # Example
    /// ```rust
    /// use fluentbase_revm::{SetGenericStage, EvmBuilder, Handler, primitives::{SpecId, HandlerCfg}};
    /// use revm_primitives::CancunSpec;
    /// use fluentbase_types::EmptyJournalTrie;
    /// let builder = EvmBuilder::<'static, SetGenericStage, (), EmptyJournalTrie>::default();
    ///
    /// // get the desired handler
    /// let mainnet = Handler::mainnet::<CancunSpec>();
    /// let builder = builder.with_handler(mainnet);
    ///
    /// // build the EVM
    /// let evm = builder.build();
    /// ```
    pub fn with_handler(
        self,
        handler: Handler<'a, EXT, DB>,
    ) -> EvmBuilder<'a, BuilderStage, EXT, DB> {
        EvmBuilder {
            context: self.context,
            handler,
            phantom: PhantomData,
        }
    }

    /// Builds the [`Evm`].
    pub fn build(self) -> Evm<'a, EXT, DB> {
        Evm::new(self.context, self.handler)
    }

    /// Register Handler that modifies the behavior of EVM.
    /// Check [`Handler`] for more information.
    ///
    /// When called, EvmBuilder will transition from SetGenericStage to HandlerStage.
    pub fn append_handler_register(
        mut self,
        handle_register: register::HandleRegister<EXT, DB>,
    ) -> EvmBuilder<'a, HandlerStage, EXT, DB> {
        self.handler
            .append_handler_register(register::HandleRegisters::Plain(handle_register));
        EvmBuilder {
            context: self.context,
            handler: self.handler,

            phantom: PhantomData,
        }
    }

    /// Register Handler that modifies the behavior of EVM.
    /// Check [`Handler`] for more information.
    ///
    /// When called, EvmBuilder will transition from SetGenericStage to HandlerStage.
    pub fn append_handler_register_box(
        mut self,
        handle_register: register::HandleRegisterBox<EXT, DB>,
    ) -> EvmBuilder<'a, HandlerStage, EXT, DB> {
        self.handler
            .append_handler_register(register::HandleRegisters::Box(handle_register));
        EvmBuilder {
            context: self.context,
            handler: self.handler,

            phantom: PhantomData,
        }
    }

    /// Sets specification Id , that will mark the version of EVM.
    /// It represent the hard fork of ethereum.
    ///
    /// # Note
    ///
    /// When changed it will reapply all handle registers, this can be
    /// expensive operation depending on registers.
    pub fn with_spec_id(mut self, spec_id: SpecId) -> Self {
        self.handler.modify_spec_id(spec_id);
        EvmBuilder {
            context: self.context,
            handler: self.handler,

            phantom: PhantomData,
        }
    }

    /// Allows modification of Evm Database.
    pub fn modify_db(mut self, f: impl FnOnce(&mut DB)) -> Self {
        f(&mut self.context.evm.db);
        self
    }

    /// Allows modification of external context.
    pub fn modify_external_context(mut self, f: impl FnOnce(&mut EXT)) -> Self {
        f(&mut self.context.external);
        self
    }

    /// Allows modification of Evm Environment.
    pub fn modify_env(mut self, f: impl FnOnce(&mut Box<Env>)) -> Self {
        f(&mut self.context.evm.env);
        self
    }

    /// Sets Evm Environment.
    pub fn with_env(mut self, env: Box<Env>) -> Self {
        self.context.evm.env = env;
        self
    }

    /// Allows modification of Evm's Transaction Environment.
    pub fn modify_tx_env(mut self, f: impl FnOnce(&mut TxEnv)) -> Self {
        f(&mut self.context.evm.env.tx);
        self
    }

    /// Sets Evm's Transaction Environment.
    pub fn with_tx_env(mut self, tx_env: TxEnv) -> Self {
        self.context.evm.env.tx = tx_env;
        self
    }

    /// Allows modification of Evm's Block Environment.
    pub fn modify_block_env(mut self, f: impl FnOnce(&mut BlockEnv)) -> Self {
        f(&mut self.context.evm.env.block);
        self
    }

    /// Sets Evm's Block Environment.
    pub fn with_block_env(mut self, block_env: BlockEnv) -> Self {
        self.context.evm.env.block = block_env;
        self
    }

    /// Allows modification of Evm's Config Environment.
    pub fn modify_cfg_env(mut self, f: impl FnOnce(&mut CfgEnv)) -> Self {
        f(&mut self.context.evm.env.cfg);
        self
    }

    /// Clears Environment of EVM.
    pub fn with_clear_env(mut self) -> Self {
        self.context.evm.env.clear();
        self
    }

    /// Clears Transaction environment of EVM.
    pub fn with_clear_tx_env(mut self) -> Self {
        self.context.evm.env.tx.clear();
        self
    }
    /// Clears Block environment of EVM.
    pub fn with_clear_block_env(mut self) -> Self {
        self.context.evm.env.block.clear();
        self
    }

    /// Resets [`Handler`] to default mainnet.
    pub fn reset_handler(mut self) -> Self {
        self.handler = Self::handler(self.handler.cfg());
        self
    }
}
