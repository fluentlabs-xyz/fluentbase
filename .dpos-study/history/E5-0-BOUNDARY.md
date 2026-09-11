# E5-0-BOUNDARY — строка 5.0 этапа Э5, заход А: одна граница модуля beacon

Ветка `djadjka/dpos-reth-2.2-squashed`, база `1e5f394e`. Сдача — рабочее дерево, не коммит.
Стенд и приватность подмодулей — заход Б, здесь не делались (решение владельца).

## §0 Прямые ответы

1. Из `beacon/` выходило 29 элементов (25 `pub` + 4 `pub(crate)`), стало 24 (20 `pub` + 2
   `pub(crate)` продакшн + 2 `pub(crate)` под `#[cfg(test)]`). Продакшн-мест вызова переведено
   30 в 9 файлах: `spec_exec` 1, `cert_inlet` 6, `outer` 1, `epoch_manager` 9, `executor` 5,
   `consensus/dpos.rs` 4 (включая crash-replay), `node/dpos.rs` 2, `node/cert_inlet.rs` 1,
   `node/consensus_rpc/state.rs` 1. [KNOWN]
2. Нет. `seed`/`terminal_seed` синхронны и отвечают только по точному раунду, `oracle_for`
   синхронен, `signer` возвращает тот же вердикт (гейт `mandatory_at` добавлен явно и НИЧЕГО не
   меняет: до `DETERMINISTIC_BOOTSTRAP_EPOCH` `carry.rs` уже отдавал `Some(None)` →
   `BeaconResolve::Absent` → `material == None`, тот же результат). Изменения поведения есть в
   двух местах ВНЕ vote/cert-пути, оба названы в §3: курсор чтения `dkgQual` и форма шатдауна
   (один drain вместо трёх). Третье — `GeometryUnfrozen` — В ПЕРВОЙ ВЕРСИИ МЕНЯЛО ВЕРДИКТ, а не
   только причину; поймано контр-ревью, починено, разбор в §10. [KNOWN]
3. lib 636/0 · node `dpos::` 8/0 · clippy без фичи 0 моих предупреждений (одно чужое,
   `large_enum_variant` на `ValidatorUpstream`, файла не касался) · clippy `fluentbase-consensus`
   с фичей 0 · fmt на тронутых файлах чисто · стенд `--no-run` собирается с фичей и без ·
   стенд прогон с фичей 35/0/0. [KNOWN]
4. Восемь отклонений от §5.1, таблица в §2. Самое крупное: `BeaconInputs` как ОДИН enum не
   вводится — два конструктора `build` / `build_follower` над одним `(Arc<dyn Beacon>, Tasks)`,
   потому что вариант `Follower` заставил бы место вызова назвать семь неиспользуемых
   generic-параметров.
5. `SeedStore` получил собственный `broadcast::Sender<BeaconEvent>`, и пробуждение
   `SeedRecorded` шлёт `record` (`beacon/certify.rs:227-234`), а не `LiveBeacon::record_seed`.
   Первая (неверная) версия слала из `record_seed` — промоутер карантина пишет через тот же
   `record`, и его записи переставали будить executor; поймано двумя красными тестами.
6. Заходу Б: приватность подмодулей (`pub(crate) mod` → `mod`) и переписывание тестовых
   реализаций стенда. НЕ сделано из захода А: `agreement_intake` остался третьим полем `Tasks`
   (перенос подметания — работа 5.4, её трогать запрещено); `faults()` без потребителя.
   Один файл пришлось тронуть вне списка — `beacon/actor.rs`, одна строка в его собственном
   тестовом модуле; подробности в §6.
7. §5.1 неверен против кода в трёх местах — перечислены в §2 как отклонения Д-2, Д-3, Д-6:
   счёт «9 методов», удаление `observe_epoch`/`observe_cert` и `broadcast` как замена
   `notify_one` без потери свойства хранимого пермита.
8. `sed -n 'Np'` и `grep ... | cut -c1-200`; строк длиннее 500 символов в тронутых `.rs` нет,
   в `.md` — да (`01_system_map.md:36`, `.claude/dpos_architecture/`), читал их через
   `sed -n '36p' … | fold -w 160`.

## §1 Инвентаризация «до»

Считано скриптом по `crates/dpos/consensus/src` и `crates/node/src` без `beacon/`; test —
это либо строка ниже первого `#[cfg(test)]` файла, либо любой файл под `testbed/`. [KNOWN]

| элемент | путь | prod | test |
|---|---|---|---|
| `Randomness` (трейт) | `surface.rs` | 8 | 2 |
| `Randomness::oracle_for` | `surface.rs` | 2 | 0 |
| `Randomness::signer_scheme` | `surface.rs` | 1 | 0 |
| `build` | `plane.rs` | 2 | 0 |
| `BeaconConfig` | `plane.rs` | 1 | 0 |
| `ArtifactSource` | `plane.rs` | 5 | 0 |
| `CommitteePairFor` | `actor.rs` | 1 | 0 |
| `CommitteeSource` | `artifact.rs` | 1 | 0 |
| `frozen_dkg_qual` | `carry.rs` | 1 | 0 |
| `agreement_partition` | `dkg_engine.rs` | 1 | 0 |
| `absent_unregistered` | `surface.rs` | 2 | 0 |
| `for_follower` | `follower.rs` | 1 | 0 |
| `FollowerBeacon` | `follower.rs` | 1 | 0 |
| `FollowerRandomnessConfig` | `follower.rs` | 1 | 0 |
| `ArtifactFetch` | `follower.rs` | 1 | 0 |
| `seed::Seed` | `seed.rs` | 7 | 7 |
| `witness_fallback_seed` | `seed.rs` | 1 | 0 |
| `constant_fallback_seed` | `seed.rs` | 2 | 1 |
| `prev_randao_from_seed` | `seed.rs` | 0 | 3 |
| `certify::SeedStore` | `certify.rs` | 1 | 32 |
| `keys::AgreedKeys` | `keys.rs` | 1 | 0 |
| `keys` (док-ссылки) | `keys.rs` | 1 | 1 |
| `actor::CommitteeFor` | `actor.rs` | 1 | 0 |
| `actor::DkgActor` | `actor.rs` | 1 | 0 |
| `actor::DETERMINISTIC_BOOTSTRAP_EPOCH` | `actor.rs` | 0 | 5 |
| `artifact::ArtifactStore` | `artifact.rs` | 1 | 0 |
| `metrics::BeaconMetrics` | `metrics.rs` | 1 | 4 |
| `dkg_engine` (док-ссылка) | `dkg_engine.rs` | 1 | 0 |
| `for_keys` | `surface.rs` | 0 | 1 |
| `for_seeds` | `surface.rs` | 0 | 2 |
| `absent` | `surface.rs` | 0 | 3 (стенд) |
| `BeaconResolve` | `surface.rs` | 0 | 3 |
| `surface::LiveBeacon::build` (тогда `PlaneRandomness`) | `surface.rs` | 0 | 3 |
| `surface::PlaneRandomnessConfig` | `surface.rs` | 0 | 2 |
| `surface::DealtOracle` | `surface.rs` | 0 | 2 |
| `surface::testing::Canned` | `surface.rs` | 0 | 5 |
| `surface::StaticRandomness` | `surface.rs` | 0 | 1 (стенд) |
| `verified_seed::VerifiedSeed` | `verified_seed.rs` | 0 (2 через `capture_certificate_seed`) | 2 |
| `verified_seed::PkOracle` | `verified_seed.rs` | 0 | 2 |
| `keys::InvalidSeed` | `keys.rs` | 1 (в `cert_inlet`) | 0 |
| `outcome::DkgOutcome` | `outcome.rs` | 0 | 1 |
| `carry::DkgQualProbe` | `carry.rs` | 0 | 1 (стенд) |

**Итог.** Из `beacon/mod.rs` выходило 29 элементов: 25 `pub`, 4 `pub(crate)`. Внешних
упоминаний по путям `crate::beacon::…` / `fluentbase_consensus::beacon::…` — 125: 48
продакшн, 77 тестовых. Восемь хэндлов + `agreement_intake` из результата `beacon::build`
считаны как одно поле каждый в `node/dpos.rs` (`:1988-2002`) — они не в этой таблице, потому что
это поля структуры, а не элементы `mod.rs`; их перечень в §2.

## §2 Граница «после»

`beacon/mod.rs` — 20 `pub` элементов, 2 `pub(crate)` продакшн (`agreement_partition`,
`absent_unregistered`), 2 `pub(crate)` под `#[cfg(test)]` (`absent`, `StaticRandomness` — их
берёт стенд). Полный список в §5.

Трейт (`beacon/surface.rs`), сокращённо:

~~~rust
pub trait Beacon: Send + Sync {
    fn seed(&self, round: Round) -> Option<Seed>;                 // sync, только точный раунд
    fn terminal_seed(&self, round: Round) -> Option<Seed>;        // sync, только пин эпохи
    fn mandatory_at(&self, epoch: u64) -> bool;                   // согласованные данные
    fn can_participate(&self, epoch: Epoch) -> ShareProbe;        // дешёвая проба
    fn signer(&self, epoch: Epoch, snap: &ValidatorSetSnapshot,
              keypair: &ValidatorBlsKeypair) -> SignerVerdict;
    fn oracle_for(&self, epoch: u64) -> Option<Arc<dyn SeedOracle>>;   // sync
    fn ensure_key(&self, epoch: u64, effort: PinEffort) -> BoxFuture<'_, bool>;
    fn observe_certificate(&self, cert: ObservedCertificate<'_>) -> Observed;
    fn artifact_bytes(&self, epoch: u64) -> Option<Vec<u8>>;
    fn observe_epoch(&self, reconciled: Epoch, entered_frontier: Epoch);  // переходное, 5.1
    fn observe_cert(&self, epoch: u64);                                  // переходное, 5.2
    fn subscribe(&self) -> broadcast::Receiver<BeaconEvent>;
    fn faults(&self) -> Option<mpsc::UnboundedReceiver<DataFault>>;
}

pub enum ObservedCertificate<'a> {
    Notarization(Round, &'a Notarization<BlsScheme, Digest>),   // σ восстановлена локально
    Finalization(Round, &'a Finalization<BlsScheme, Digest>),   // σ пришла с провода
}
pub enum Observed { Recorded, Pending, Refused, Inactive }
pub enum BeaconEvent { SeedRecorded, KeyAvailable, ParticipationChanged }
pub struct DataFault { pub epoch: u64, pub refused: usize }
~~~

Входы и задачи (`beacon/plane.rs`, `beacon/follower.rs`):

~~~rust
pub trait CommitteeReads: Send + Sync {
    fn read_at(&self) -> Option<B256>;                                   // ОДИН курсор
    fn committee(&self, epoch: u64, at: B256) -> Option<Set<PeerPubkey>>;
    fn committee_bls(&self, epoch: u64, at: B256) -> Option<EpochCommittee>;
    fn dkg_qual(&self, epoch: u64, at: B256) -> Option<(bool, bool)>;
    fn committee_pair(&self, target: u64)                                // provided: один хэш
        -> Option<(Set<PeerPubkey>, Set<PeerPubkey>)>;
}

pub struct ValidatorInputs<P, Se, Re, XS, XR, HS, HR> {
    chain_id, peer_keypair, bls_keypair, share_dir, share_seal_key,
    peers, beacon_channel, resolver_channel,
    vote_mux, cert_mux, resolver_mux, bodies_mux,
    committees: Arc<dyn CommitteeReads>,
    heights: mpsc::Receiver<u64>,
    plane_clock: PlaneClock,
    geometry: watch::Receiver<Option<(u64, u64)>>,   // None = Unfrozen, не ошибка
    partition_prefix: String,
}

pub struct FollowerInputs {
    chain_id, committees: Arc<dyn CommitteeReads>, fetch: ArtifactFetch,
}   // без ключей, муксов, DKG и partition_prefix — follower RAM-only по решению

pub struct Tasks { pub supervised: Handle<()>, pub drain: Handle<()>,
                   pub agreement_intake: mpsc::Receiver<(Epoch, Handle<()>)> }

pub async fn build<..>(&E, ValidatorInputs<..>) -> eyre::Result<(Arc<dyn Beacon>, Tasks)>;
pub fn build_follower<E>(&E, FollowerInputs) -> (Arc<dyn Beacon>, Tasks);
~~~

### Отклонения от §5.1

| пункт | что иначе | file:line причины |
|---|---|---|
| Д-1. `BeaconInputs` как один enum | Два конструктора над одним `(Arc<dyn Beacon>, Tasks)`: `build(ValidatorInputs)` и `build_follower(FollowerInputs)` | Вариант `Validator` несёт семь generic-параметров p2p (`plane.rs:377-383` — `P, Se, Re, XS, XR, HS, HR`), у `Follower` нет ни одного. Один enum заставил бы место вызова follower-а (`consensus/dpos.rs:3517`) назвать их турбофишем. Граница от этого не шире: обе двери отдают один и тот же тип пары |
| Д-2. «трейт из 9 методов» | 11 запросов/приёмов + `subscribe` + `faults` | `ensure_key` — 9-й: §5.1 отправляет его внутрь, но лестница ключей epoch_manager-а зовёт его явно (`epoch_manager.rs:1713-1716`), а сделать приобретение чисто реактивным — работа 5.1. `observe_epoch`/`observe_cert` — 10-й и 11-й: §5.1 удаляет их «после 5.1 и 5.2», а сегодня они ведут ретенцию (`surface.rs:2564-2596`), и без них она пропадёт |
| Д-3. `subscribe` без потерь | `broadcast`, но с явным правилом «подписаться ДО первого чтения» | §5.1 требует и `broadcast`, и сохранение свойства хранимого пермита `notify_one`. Оба сразу невозможны: broadcast буферизует с момента подписки. Правило записано в трейте и соблюдено на обоих потребителях (`executor.rs:1168` до первой пробы σ, `epoch_manager.rs:675` до первого reconcile) |
| Д-4. `BeaconEvent` с полезной нагрузкой | Три варианта без нагрузки | Два из трёх производителей её не имеют: `BeaconKeys::set_pk` и участие пишут голый `Notify` из задач, которые эпоху не называют. Потребитель всё равно перечитывает состояние, а `Lagged` схлопывает два события в одно |
| Д-5. `Stalled{epoch, reason}` в событиях | Не введён | Производителя нет до 5.3: состояния `Unrecoverable`/`Conflict`/`Stalled` — её работа. Вариант без конструктора — мёртвая поверхность |
| Д-6. `agreement_intake` уходит внутрь | Остался третьим полем `Tasks` | Потребитель — `epoch_manager::prune_agreements` (`epoch_manager.rs:318-366`), его перенос вместе с band-sweep, join-семантикой и защёлкой SafetyHalt — явная работа 5.4, начинать которую запрещено. Убрать приёмник без подметания = инстансы перестанут пруниться |
| Д-7. Одна боевая реализация `LiveBeacon` | Две за одним трейтом: `LiveBeacon` и `FollowerRandomness` | Слияние — артефактная половина `follower.rs`, названная работой 5.1. Решение владельца «потребители держат `Arc<dyn Beacon>`» соблюдено: обе выдаются как `Arc<dyn Beacon>`, никто снаружи их не различает |
| Д-8. `faults()` с потребителем | Канал есть, продюсер есть (промоутер), потребителя нет | Ротация inlet-а по `DataFault` — работа 5.2 (Д-3 вариант (в)). Чтобы неподписанный канал не рос, отправка гейтится флагом `faults_armed`, который поднимает только первый `faults()` (`surface.rs:2205-2214`) |

## §3 Потребители

| потребитель | старые вызовы | новые | изменение поведения |
|---|---|---|---|
| `spec_exec` | `oracle_for` + `VerifiedSeed::check` + `record_seed`/`quarantine_seed` + `error!`, `spec_exec.rs:97-116` (до) | `observe_certificate(Notarization)`, `spec_exec.rs:91-93` | ничего: ветка `Notarization` в `surface.rs` повторяет прежние три исхода дословно, включая `error!` про локально восстановленную σ |
| `cert_inlet::ingest` | `capture_certificate_seed(...)`, `cert_inlet.rs:866` (до) | `observe_certificate(Finalization)`, `cert_inlet.rs:860-862` | ничего: ветка `Finalization` сохраняет консультацию `on_invalid_seed` и три её исхода |
| `cert_inlet::UpstreamResolver` | `capture_certificate_seed(...)`, `cert_inlet.rs:3154` (до) | `observe_certificate(Finalization)`, `cert_inlet.rs:3129-3133` | ничего; `observe_cert` по-прежнему НЕ зовётся на этой двери |
| `cert_inlet` (ключи) | `ensure_key`, `oracle_for`, `mandatory_at`, `observe_cert` | те же, `cert_inlet.rs:687,725,818,884` | ничего |
| `executor` (σ) | `seed_for`, `executor.rs:2635,2923,2934` (до) | `seed`, `executor.rs:2648,2936,2947` | ничего: переименование |
| `executor` (пробуждение) | `seed_edge()` + `notified()`, `executor.rs:1167,1483` (до) | `subscribe()` + фильтр `SeedRecorded`, `executor.rs:1168,1490-1499` | ничего по исходу: два чужих класса отфильтрованы `continue`, свой класс шлёт `SeedStore::record` — тот же писатель, что фиксировал `notify_one` |
| `epoch_manager` (проба/подпись) | `share_probe` ×2, `signer_scheme`, `epoch_manager.rs:1071,1127,1198` (до) | `can_participate` ×2, `signer`, `epoch_manager.rs:1094,1150,1221` | ничего: позиции те же (перед граничным чтением), вердикты те же |
| `epoch_manager` (граничная база) | `terminal_seed_at`, `epoch_manager.rs:163` | `terminal_seed`, там же | ничего |
| `epoch_manager` (пробуждения) | два плеча `select!`: `participation_edge` и `key_edge`, `epoch_manager.rs:666,673,771,818` (до) | одно плечо над `subscribe()`, `epoch_manager.rs:675,808-828` | **ИЗМЕНЕНИЕ, названо:** объединённое плечо на приход share теперь ещё и будит sweep (`sweep_wake.send_replace`), чего плечо `share_n` не делало. Sweep идемпотентен и построен, чтобы его будили; альтернатива — два плеча, которые обязаны совпадать вручную |
| `epoch_manager` (лестница/ретенция) | `ensure_key`, `oracle_for`, `observe_epoch` | те же, `epoch_manager.rs:1713-1716,1575,1022` | ничего |
| `outer`/`engine`/`scheme` | `Arc<dyn Randomness>`, `oracle_for` | `Arc<dyn Beacon>`, `oracle_for`, `outer.rs:536,750,1192` | ничего: тип двери |
| crash-replay (`consensus/dpos.rs`) | `mandatory_at` + `seed_for`, `dpos.rs:575-590` | `mandatory_at` + `seed`, `dpos.rs:575-590` | ничего. `ReplaySeed::Defer` через `observe_certificate` — работа 5.2 |
| `consensus/dpos.rs::launch_follower` | `for_follower(FollowerRandomnessConfig{committees, dkg_qual, fetch})` + `FollowerBeacon{..}` | `build_follower(FollowerInputs{chain_id, committees, fetch})`, `dpos.rs:3517-3536` | **ИЗМЕНЕНИЕ, названо:** `dkgQual` follower-а читается на `CommitteeReads::read_at()` (finalized-хэш) — тот же хэш, что и раньше, других читателей у follower-а нет |
| `node/dpos.rs` (сборка) | `BeaconConfig` с 4 замыканиями + `dkg_qual_at`/`dkg_qual_probe` | `ValidatorInputs{committees: Arc<dyn CommitteeReads>}`, `node/dpos.rs:1366-1478` | **ИЗМЕНЕНИЕ, названо:** `dkgQual` читается на `max(fin, live)` вместо finalized-хэша. Бит монотонный и замораживается memo, курсор cert-финализирован (без реорга), поэтому единственный эффект — увидеть УЖЕ выставленный бит раньше; выставленный не может «разставиться» |
| `node/dpos.rs` (геометрия) | `Box::pin(async { geometry_ready.notified().await; frozen_geometry() })` | `watch::Receiver<Option<(u64,u64)>>`, `node/dpos.rs:1560-1571,1697-1706` | **ИЗМЕНЕНИЕ, названо и намеренное:** гонка одноразового `Notify` с чтением `frozen_geometry()` больше не оставляет узел без `DkgActor` на весь процесс. `GeometryUnfrozen` — второе имя для ТОГО ЖЕ удержания, выбираемое только там, где материала и так нет (`surface.rs`, внутри арма `material.is_none()`); вердикт совпадает с прежним всегда. Как именно первая версия это нарушала — §10 |
| `node/dpos.rs` (супервизия) | 5 supervised + 3 drain хэндла, `node/dpos.rs:831-839,856-871` (до) | `("beacon", tasks.supervised)` и `("beacon", tasks.drain)`, `node/dpos.rs:790-800,820` | **ИЗМЕНЕНИЕ, названо:** три писателя журналов ждутся ПАРАЛЛЕЛЬНО внутри одного 5-секундного лимита узла вместо трёх последовательных по 5 с. Строки лога на писателя сохранены. Кто из шести supervised-детей умер — в строке, которую пишет супервизор beacon-а |
| `node/consensus_rpc/state.rs` | `beacon::ArtifactSource` над `ArtifactStore` | `dpos::ArtifactSource` над `Beacon::artifact_bytes` через `Weak`, `node/dpos.rs:1905-1917`, `consensus/dpos.rs:3526-3538` | ничего — но `Weak` здесь обязателен, и первая версия его не имела: §10 |
| `node/cert_inlet.rs` | `Arc<dyn Randomness>` | `Arc<dyn Beacon>` | ничего |

## §4 Стенд

Тронуто ради компиляции (`testbed/`):

- `stand.rs`: `BeaconConfig` → `ValidatorInputs`; четыре замыкания и `dkg_qual_probe` заменены
  одной локальной реализацией `CommitteeReads` (`StandCommitteeReads`), `read_at` — то же
  `max(EL-fin, live)` с тем же откатом; `geometry` — `watch`; три поля прежнего результата
  сборки заменены парой `(beacon, beacon_tasks)`; локальный `type ArtifactSource` вместо
  удалённого из `beacon`; `CommitteeFor` и `roster` теперь под фичей `dpos-devnet-byzantine`
  (единственный оставшийся потребитель — роль `TwoReveals`).
- `byzantine_roles.rs`: `WithholdingRandomness` переписан с `impl Randomness` на `impl Beacon`
  над `Arc<dyn Beacon>` — 12 делегирований и одна изменённая операция (`signer`). Иначе
  обёртке нужен был бы внутренний трейт, который снаружи `beacon/` не виден.

Заходу Б: приватность подмодулей (`pub(crate) mod` → `mod`) и переписывание `StaticRandomness`,
`Absent`, `Canned` и обёртки роли под трейт — то есть всё, что §5.1 называет «швом стенда».
Сегодня они по-прежнему реализуют внутренний `Randomness` и получают `Beacon` бесплатно через
blanket-impl.

## §5 `pub` элементы `beacon/mod.rs` после

`build_follower`, `ArtifactFetch`, `FollowerInputs`, `build`, `CommitteeReads`, `Tasks`,
`ValidatorInputs`, `constant_fallback_seed`, `prev_randao_from_seed`, `witness_fallback_seed`,
`Seed`, `Beacon`, `BeaconEvent`, `DataFault`, `Observed`, `ObservedCertificate`, `PinEffort`,
`ShareProbe`, `SignerVerdict`, `WithheldReason` — 20.

`pub(crate)`: `agreement_partition`, `absent_unregistered` (продакшн); `absent`,
`StaticRandomness` (под `#[cfg(test)]`, для стенда) — 4.

## §6 Ворота

~~~
$ cargo test -p fluentbase-consensus --lib
test result: ok. 636 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 20.90s

$ cargo test -p fluentbase-node dpos::
test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 49 filtered out; finished in 0.00s

$ cargo clippy -p fluentbase-consensus -p fluentbase-node --all-targets
warning: large size difference between variants
    --> crates/node/src/dpos.rs:1945:1
warning: `fluentbase-node` (lib) generated 1 warning
warning: `fluentbase-node` (lib test) generated 1 warning (1 duplicate)

$ cargo clippy -p fluentbase-consensus --all-targets --features dpos-devnet-byzantine
(0 строк error/warning)

$ cargo test -p fluentbase-consensus testbed:: --no-run                      # без фичи
    Finished `test` profile [unoptimized + debuginfo] target(s) in 30.80s
$ cargo test -p fluentbase-consensus testbed:: --features dpos-devnet-byzantine --no-run
    Finished `test` profile [unoptimized + debuginfo] target(s) in 43.21s

$ cargo test -p fluentbase-consensus testbed:: --features dpos-devnet-byzantine
test result: ok. 35 passed; 0 failed; 0 ignored; 0 measured; 609 filtered out; finished in 31.66s

$ rustfmt --edition 2021 $(git diff --name-only | grep '\.rs$')
$ cargo fmt -p fluentbase-consensus -p fluentbase-node -- --check
(ни одной строки `Diff in`)
~~~

Единственное предупреждение clippy — `large_enum_variant` на `ValidatorUpstream`
(`node/dpos.rs:1945`). Ни `ValidatorUpstream`, ни `plane_upstream.rs` в диффе нет
(`git diff --name-only`), значит оно чужое: дрейф версии clippy против даты замера в PLAN.

**[СНЯТО как устаревшее, заход В]** Здесь стояло: «Клипи с фичей на `fluentbase-node` не
запускается и без моих правок: `DposLayerConfig` получает под фичей поле `byzantine`,
которого узел не заполняет (`E0063` на `crates/node/src/dpos.rs:2176`)». Это неверно и было
опровергнуто уже заходом Б (Б§8): фичевый clippy собирается и чист. Фичевые ворота стоят на
`fluentbase-consensus` по другой причине — фича объявлена там, и `--all-targets` на этом
крейте покрывает стенд, ради которого она и существует.

~~~
$ git status --short | grep -v '^!!'
 M .dpos-study/PLAN.md
 M crates/dpos/consensus/src/application.rs
 M crates/dpos/consensus/src/beacon/actor.rs
 M crates/dpos/consensus/src/beacon/carry.rs
 M crates/dpos/consensus/src/beacon/certify.rs
 M crates/dpos/consensus/src/beacon/follower.rs
 M crates/dpos/consensus/src/beacon/keys.rs
 M crates/dpos/consensus/src/beacon/metrics.rs
 M crates/dpos/consensus/src/beacon/mod.rs
 M crates/dpos/consensus/src/beacon/plane.rs
 M crates/dpos/consensus/src/beacon/resolve.rs
 M crates/dpos/consensus/src/beacon/surface.rs
 M crates/dpos/consensus/src/cert_inlet.rs
 M crates/dpos/consensus/src/dpos.rs
 M crates/dpos/consensus/src/engine.rs
 M crates/dpos/consensus/src/epoch_manager.rs
 M crates/dpos/consensus/src/executor.rs
 M crates/dpos/consensus/src/outer.rs
 M crates/dpos/consensus/src/scheme.rs
 M crates/dpos/consensus/src/spec_exec.rs
 M crates/dpos/consensus/src/testbed/byzantine_roles.rs
 M crates/dpos/consensus/src/testbed/stand.rs
 M crates/node/src/cert_inlet.rs
 M crates/node/src/consensus_rpc/state.rs
 M crates/node/src/dpos.rs
~~~

**Один файл вне разрешённого списка: `crates/dpos/consensus/src/beacon/actor.rs`.** Правка —
одна строка в его собственном `#[cfg(test)] mod clock_tests` (`:7473`): путь
`crate::beacon::BeaconResolve` → `crate::beacon::surface::BeaconResolve`, потому что
ре-экспорт `BeaconResolve` из парадной двери удалён. Без неё крейт не собирается с тестами.
Поведения она не меняет.

Ещё четыре файла в списке не названы, но правки в них того же класса и по одной-две строки:
переезд импорта `BeaconResolve` (`beacon/resolve.rs`), док-ссылки на переименованные методы
(`engine.rs:93`, `scheme.rs:53`) и снятие поля `participation` из тестовой конфигурации
(`application.rs`). `beacon/carry.rs` в списке есть — там переименование в комментарии и
переписанная оговорка про `Some(None)`, которая теперь честно говорит, что второй потребитель
на неё больше не опирается.

`.dpos-study/PLAN.md` отслеживается git-ом и потому виден в статусе; `.claude/dpos_architecture/*`
и остальное под `.dpos-study/` — нет (`.claude` и `.dpos-study` в `.gitignore`).

## §7 Что всплыло и что не сделано

**Для 5.1.** `carry::chain_key_epoch_memoised`'s `Some(None)` больше не единственная дорога к
«у пре-beacon эпохи нет материала» — в `signer_scheme` теперь стоит явный гейт
`mandatory_at` (`surface.rs:2405-2418`), так что удаление `carry.rs` не меняет вердикт.
`frozen_dkg_qual` и `CommitteeSource` больше не покидают `beacon/` вообще (потребителей вне
модуля нет) — их удаление в 5.1 не заденет ни одного внешнего файла.

**Для 5.2.** `faults()` ждёт потребителя: продюсер (промоутер карантина, `plane.rs:958-960`)
шлёт `DataFault{epoch, refused}` рядом с существующей ERROR-строкой, но только когда приёмник
взят. `SeedStore::wait_for`/`prune_waiters` оказались тестовыми ещё до этой работы
(`git grep '\.wait_for(' HEAD` — один вызов, в собственном тесте `certify.rs`); они помечены
`#[cfg(test)]`, потому что после сужения `for_seeds` до теста их перестала прикрывать
публичная сигнатура. 5.2 удаляет их вместе с `waiters`.

**Для 5.4.** `Tasks::agreement_intake` — единственная часть парадной двери, которую заход А не
закрыл; см. Д-6.

**Не сделано из захода А, с причиной.** Приватность подмодулей и переписывание тестовых
реализаций — прямое указание владельца («заход Б, здесь не делать»). Событие
`Stalled{epoch, reason}` — нет производителя до 5.3. Перевод crash-replay на
`observe_certificate` с `Pending ⇒ Defer` — это 5.2 (`ReplaySeed::Defer` по `KeyAvailable`).

**[СНЯТО как устаревшее, заход В]** Здесь стояло: «`cargo clippy` на `fluentbase-node` с
фичей `dpos-devnet-byzantine` не собирается на HEAD … фичевые ворота узла сегодня
непроверяемы». Опровергнуто заходом Б (Б§8) и перепроверено заходом В: собирается и чист.

## §8 Логические коммиты

Собираются сами по себе в этом порядке; проверено тем, что каждая следующая группа опирается
только на предыдущие.

| путь | группа | заголовок |
|---|---|---|
| `beacon/{surface,certify,metrics}.rs` | 1 | `refactor(beacon): give the beacon one trait and one wake-up publisher` |
| `beacon/{plane,follower,mod,keys,resolve}.rs` | 2 | `refactor(beacon): build the beacon behind two constructors, one CommitteeReads and two task handles` |
| `consensus/src/{spec_exec,cert_inlet,executor,epoch_manager,outer,engine,scheme,application,dpos}.rs` | 3 | `refactor(consensus): move every beacon consumer onto the Beacon trait` |
| `node/src/{dpos,cert_inlet,consensus_rpc/state}.rs` | 4 | `refactor(node): hand the beacon one CommitteeReads and supervise it as one subsystem` |
| `consensus/src/testbed/{stand,byzantine_roles}.rs` | 5 | `test(testbed): follow the beacon boundary` |
| `.claude/dpos_architecture/*`, `.dpos-study/*` | 6 | `docs(dpos): record the beacon boundary` |

Оговорка: группы 1 и 2 по отдельности НЕ компилируются — трейт и его единственные
реализации живут в двух файлах, которые ссылаются друг на друга (`plane.rs` строит
`LiveBeacon`, `surface.rs` берёт `ArtifactStore` и `watch` из его входов). Их надо слить в
один коммит, либо принять, что зелёная точка — после группы 2. Группы 3-6 компилируются
каждая.

## §10 Контр-ревью: что оно нашло и что из этого починено

Четыре ревьюера в свежем контексте, каждый на своей области, read-only. Ограничение, которое
стоит записать на будущее: агенту типа `reviewer` доступны только Read/Grep/Glob, без Bash —
поэтому ни один из четверых НЕ смог выполнить сверку `git show HEAD:` с прежними телами, хотя
трое получили её как явное задание. Сверки с базой делал я сам, по каждой находке отдельно.

**Два дефекта, оба мои, оба подтверждены моим собственным чтением и починены.**

**(1) `GeometryUnfrozen` менял вердикт, а не только причину.** Ветка стояла ПЕРЕД чтением
`material`. Но `share_state::load_all` наполняет ceremony-стор с диска внутри `build`
(`beacon/plane.rs:592`), не спрашивая геометрию, а watch геометрии стартует `None`
(`node/dpos.rs:1514`) и публикуется только после удачного `cold_start` поллера (`:1628`).
Значит у валидатора, перезапущенного посреди эпохи со своей шарой на диске, `material` был
`Key(..)`, а проба отвечала `Withheld`. И это не самоизлечивалось: `reconcile_roles` делает
`soft_enter` и `return`, НЕ пополняя `deferred_spawns` (`epoch_manager.rs:1150`), так что ребро
`spawn_unblocked` остаётся закрытым своим гейтом, а `ParticipationChanged` публикует только мост
от `share_notify` (`plane.rs:990`) — на разморозку геометрии не публикует никто. Узел сидел бы
verify-only до следующей границы эпохи. Починено: ветка стала УТОЧНЕНИЕМ арма `material.is_none()`,
а не гейтом перед ним, поэтому вердикт совпадает с прежним всегда, а путь восстановления —
существующий (церемония наполняет стор → `share_notify` → `ParticipationChanged`).

**(2) Замыкание `artifact_bytes` держало живой `Arc<dyn Beacon>` в RPC-реестре reth.** На HEAD
оно замыкалось на `ArtifactStore` (`git show HEAD:beacon/plane.rs:662-665`), то есть утаскивало в
долгоживущий реестр отправителя ОДНОГО журнала — артефактного. Моя версия замкнулась на весь
`Arc<dyn Beacon>`, а в нём сидят отправители всех трёх. Реестр живёт весь процесс
(`node/dpos.rs:2250` → `FeedStateHandle.artifacts`), `drop(plane.shared)` до него не дотягивается,
и каждый штатный шатдаун валидатора выжигал бы полный `SHUTDOWN_DRAIN_TIMEOUT` с предупреждением,
чья задача — означать «диск застрял». То есть я расширил предсуществующую течь с одного писателя
на трёх и при этом переформулировал предпосылку так, будто её нет. Починено: замыкание держит
`Weak`, неудачный `upgrade` отвечает `None` — правильный ответ для уходящего узла. Ту же правку
получил путь follower-а.

**Что подтвердилось прямыми ответами.** Drop-гвардия супервизора держит: `Handle::abort` будит
задачу, `Abortable::poll` отдаёт `Err(Aborted)` не опрашивая внутреннюю future, а сама `Abortable`
владеет ею и дропается до `tree.abort()` — то есть drop-glue выполняется и шесть детей
аборчатся. Executor не может потерять раннее пробуждение: `awaiting_seed` заполняется только
в `on_finalized_block` и сразу читает стор, а `subscribe()` взят до цикла — порядок, дающий вечное
удержание, непостроим. Мост не крадёт пробуждение у промоутера (`BeaconKeys::subscribe` даёт
свой `Arc<Notify>`), и пермит проигравшего плеча `select!` не теряется (tokio возвращает его при
дропе `Notified`). Асимметрия `observe_cert` между двумя ингрессами сохранена.

**Пять расхождений текста и кода, все мои, все починены:** док `EagerTrigger` обосновывал повтор
хранимым пермитом, которого больше нет (`executor.rs`); якорь в ORDERING-CRITICAL комментарии
`spec_exec` указывал в код re-jump; доки sweep в `epoch_manager` ссылались на плечи `share_n` и
`key_edge`, удалённые слиянием; парадная дверь в `beacon/mod.rs` называла `CommitteePairFor`
публичным экспортом, которого там нет; комментарий поллера ссылался на удалённый `geometry_ready`.
Отдельно исправлен переобещающий комментарий про «один курсор by construction» в `plane.rs`:
`committee_for` и `dkg_qual_for` зовут `read_at()` независимо, так что структурная атомарность
есть только внутри `committee_pair` — что Д-7 и требует, но формулировка была шире правды.

**Стенд перестал упрощать.** `StandCommitteeReads::committee`/`committee_bls` игнорировали
переданный `at` и заново резолвили курсор, то есть `committee_pair` в стенде читал обе половины
двумя независимыми чтениями — ровно то анти-straddle-свойство, ради которого трейт и вводился,
стенд не воспроизводил. Теперь читают по переданному хэшу.

**Тест drain-а признан выродившимся и переписан.** Оба ревьюера независимо сказали, что
`dropping_the_host_provider_clone_releases_the_journal_writer` после моей переделки проверял
семантику tokio-канала, а не устройство узла, и что дефект (2) жил в продакшне при зелёном тесте.
Согласен. Тест переименован в `a_drain_finishes_only_once_the_last_sender_is_gone`, стал
двусторонним (отрицательное направление: писатель обязан оставаться припаркованным, пока
отправитель жив), а его док теперь прямо перечисляет, чего он НЕ достаёт: продакшн-порядок
`drop(plane.shared)` перед ожиданием drain-а и класс «клон утёк в долгоживущий реестр».

**Принято без правки, с обоснованием.** Общий 5-секундный таймаут на трёх писателей вместо трёх
по 5 с: на устройстве, где fsync сериализуется, три честных флаша могут не уложиться. Оставлено —
работа на писателя ограничена сотнями 68-байтных append-ов и одним fsync, запас велик, а изменение
названо в §3. Два `[СЛАБАЯ]` про курсор (`live` — высота order-блока, а не EL; откат
`or_else(block_hash(fin))` обходит собственный guard при `fin == None && live > 0`) — обе ветки
предсуществуют изменению, ни один ревьюер не смог довести их до неверного значения; вынесены в §7
как наблюдения, чинить их здесь запрещено правилом «нашёл предсуществующий дефект — не чини».
Мёртвое поле `randomness` в `application.rs` — предсуществующее: на HEAD его тоже никто не читал
(`git show HEAD:application.rs` — только `clone()` и сеттер).

## §9 Где проверка была слабее всего

1. **Историю `verified-against` я переименовал механически.** Скрипт заменил
   `Randomness`→`Beacon`, `PlaneRandomness`→`LiveBeacon` и т.п. ВО ВСЁМ
   `.claude/dpos_architecture/`, включая датированные записи 2026-08-*. Это делает их
   грепаемыми, но записи теперь описывают прошлое сегодняшними именами. Новая запись за
   2026-09-11 называет все дельты явно; отдельного решения «историю не трогать» я не спрашивал.
2. **Якоря `file:line` в таблицах `09_followers.md` §9.x не перепинены.** Их около сорока,
   все сдвинулись. Вместо перепинивания добавлена пометка `[REVISED 2026-09-11]`, что якоря
   предшествуют переносу границы. Семантические колонки исправлены.
3. **`faults()` без потребителя не проверен ничем, кроме компиляции.** Продюсер существует,
   флаг `faults_armed` тестом не покрыт.
4. **Параллельный drain измерен только компиляцией и `dpos::` тестами.** Что три писателя
   действительно укладываются в 5 с на живом узле — не наблюдалось.
5. **`Drop`-гвардия супервизора не покрыта тестом.** Что `handle.abort()` на
   `Tasks::supervised` роняет шестерых детей, прочитано в `runtime/src/utils/handle.rs:57-79`,
   `futures-util/src/abortable.rs:139-141,192-195` и подтверждено контр-ревью — но прогоном не
   проверено. Каскад к тому же асинхронный: нужен ещё проход планировщика на супервизора и по
   одному на каждого ребёнка.
6. **Правка (2) из §10 — `Weak` в замыкании `artifact_bytes` — структурная, не тестовая.**
   Ни один тест не покраснеет, если кто-то вернёт туда сильный клон. Достать это свойство юнитом
   нельзя: нужен собранный reth-узел с RPC-реестром.
7. **Ни один из четырёх ревьюеров не смог сверить код с базой** — у агента типа `reviewer` нет
   Bash. Сверки с `git show HEAD:` делал я сам и только по конкретным находкам, а не сплошняком.

**Команда чтения длинных строк.** `sed -n 'Np' file` и `sed -n 'Np' file | fold -w 160`;
`cut -c1-N` применялся только к выводу `grep` для сокращения экрана, не к чтению.

**Счётчики текстовых проверок** на этом журнале, после правок: строк, оканчивающихся на
многоточие — 0; строк с нечётным числом обратных кавычек — 0; удвоенных запятых — 0; пустых
пар обратных кавычек — 0; пустых круглых скобок вне блоков кода — 0. На дописанных блоках
`.claude/dpos_architecture/` те же пять счётчиков дают 0; унаследованные строки этих файлов не
пересчитывались — там висячие открытые кавычки на переносе строки давняя норма.

---

# Часть Б — строка 5.0, заход Б: тестовые реализации на трейте, приватные подмодули

База та же, `1e5f394e`; заход А по-прежнему не закоммичен, поэтому `git diff` в дереве —
это А+Б вместе. Сдача — рабочее дерево.

## Б§0 Прямые ответы

1. lib `fluentbase-consensus` 636/0 (в них 27 стендовых без фичи, отдельным прогоном
   `--lib testbed::` — 27/0) · стенд с `dpos-devnet-byzantine` 35/0 за **31,32 с**, не ~190 с ·
   `fluentbase-node` 57/0 · clippy обеих крейтов без фичи — одно предупреждение,
   `large_enum_variant` на `ValidatorUpstream` (`node/dpos.rs:1958`), предсуществующее — сам
   файл в диффе есть, но `ValidatorUpstream` не встречается ни в одном хунке; clippy `fluentbase-consensus --all-targets --features dpos-devnet-byzantine` — ноль
   строк · `cargo fmt --check` чисто. [KNOWN]
2. Нет, ни одного. `git diff` по `testbed/tests.rs` — 2 хунка: блок `use` и одна строка
   док-комментария (`absent::seed_for` → `absent`'s `Beacon::seed`), утверждений там нет.
   В остальных файлах тронуты только пути и сборка фикстур.
   **[ИСПРАВЛЕНО, заход В (F-25)]** Здесь стояло «единственная правка внутри тела теста —
   `Randomness::ensure_key(&canned, …)` → `Beacon::ensure_key(&canned, …)`». Их было
   четыре, и greп по `assert` видел только три: помимо `surface.rs:1954-1955` —
   `surface.rs:1583` и `follower.rs:1015` (те же переименования вызова), а ЧЕТВЁРТАЯ и
   единственная содержательная — целиком переписанный тест дренажа в `node/dpos.rs`, где
   сменились фикстура (реальный `for_keys` над `BeaconKeys::with_persistence` → `Arc<dyn Any>`
   над голым сендером), имя и оба текста сообщений. Смену фикстуры греп по `assert` не ловит
   в принципе, поэтому «утверждения не тронуты, подтверждено машинно» (Б§8) верно только для
   строк `assert*`. Про три переименования формулировка остаётся точной: изменён ВЫЗОВ,
   утверждаемое значение и вердикт — нет. [KNOWN]
3. Шесть строк `use` в `beacon/mod.rs`: четыре `pub use` (20 имён — тот же список, что в А§5)
   и две `pub(crate) use` (`agreement_partition`, `absent_unregistered`). От §5.1 список
   отличается ровно восемью отклонениями захода А (А§2, Д-1…Д-8) — заход Б ни одного имени в
   продакшн-часть двери не добавил и ни одного не убрал. Изменилась ТЕСТОВАЯ часть: плоские
   `#[cfg(test)] pub(crate) use surface::{absent, StaticRandomness}` заменены одним модулем
   `#[cfg(test)] pub(crate) mod testing` с 25 именами (+6 под фичей). [KNOWN]
4. Полный список — таблица в Б§3, и читать надо её: сокращения «те же плюс» я сначала
   написал неверно (`epoch_manager` НЕ берёт ни `for_seeds`, ни `VerifiedSeed`; у
   `application` 7 имён, а не 9, как у `dpos`). Коротко: `executor` — `SeedStore`,
   `for_seeds`, `VerifiedSeed`, `PkOracle`, `DETERMINISTIC_BOOTSTRAP_EPOCH`;
   `epoch_manager` — `SeedStore`, `PkOracle`, `DETERMINISTIC_BOOTSTRAP_EPOCH`,
   `LiveBeacon`/`LiveBeaconConfig`/`BeaconResolve`, `BeaconKeys`/`AgreedKeys`/`KeySource`/
   `KeySources`, `ArtifactStore`, `BeaconMetrics`; `cert_inlet` — `Canned`, `DealtOracle`,
   `LiveBeacon`/`LiveBeaconConfig`/`BeaconResolve`, `SeedStore`, `BeaconKeys`/`AgreedKeys`/
   `AgreedKeyAt`, `DkgQualFor`, `encode_outcome`/`parse_outcome`/`group_public_key`,
   `ArtifactStore`, `BeaconMetrics`; `dpos` — весь набор `LiveBeacon`-фикстуры;
   `application` — те же без `PkOracle` и `DETERMINISTIC_BOOTSTRAP_EPOCH`; `outer` — только
   `DETERMINISTIC_BOOTSTRAP_EPOCH`; `byzantine` —
   `DkgOutcome`; `slasher/evidence` — `DealtOracle`. Вход один — модуль
   `crate::beacon::testing`, объявленный как `#[cfg(test)] pub(crate) mod testing`. [KNOWN]
5. `beacon/surface.rs:305-366` — вынос вердикта в `certificate_verdict`. Это единственная
   правка, которая ПЕРЕПИСЫВАЕТ тело, исполняемое на продакшн-пути ingress-а (blanket-impl
   зовёт её), а не переставляет пути; всё остальное в заходе Б — видимость и импорты.
6. Внутри строки 5.0 остались две вещи, обе названы в PLAN: `for_keys`/`for_seeds` не удалены
   (§5.1 требует их удаления, но `for_seeds` — вход фикстуры executor-а, а `for_keys` — двух
   тестов `surface.rs`; их удаление осмысленно вместе с 5.1/5.2, которые заменяют то, над чем
   они строятся) и `Tasks::agreement_intake` (заход А, Д-6: подметание инстансов — работа 5.4).
7. §5.1 требует «`absent`/`StaticRandomness` — тестовые реализации трейта, уезжают под
   `cfg(test)`». В коде `absent_unregistered` — НЕ тестовая: она стоит в двух продакшн-
   конструкторах (`cert_inlet.rs:500`, `application.rs:377`) как значение по умолчанию до
   `with_randomness`. Поэтому под `cfg(test)` уехала только `absent` (обёртка с регистрацией
   метрик), а `absent_unregistered` осталась в `pub(crate)`-тире. Ещё: журнал А §4 говорит, что
   заходу Б остались «`StaticRandomness`, `Absent`, `Canned` и обёртка роли» — обёртка
   (`WithholdingRandomness`) была переписана уже в А (А§4, второй пункт), так что здесь
   переписаны три, а не четыре.
8. `awk 'length($0)>500 {print FILENAME":"NR": "length($0)}'` по тронутым файлам: в `.rs`
   таких строк НЕТ ни одной; в `.md` есть (`PLAN.md:107` — 1172 символа, строка 5.0), читал
   их `sed -n '107p' .dpos-study/PLAN.md | fold -w 160`.

## Б§1 Переписанные реализации

Все три жили на внутреннем `Randomness` и получали `Beacon` через blanket-impl; теперь
реализуют `Beacon` напрямую, а `Randomness` сузился до `pub(super)` и до двух продакшн-
реализаций (`LiveBeacon`, `FollowerRandomness`).

| реализация | старые методы | новые | что изменилось в семантике фикстуры |
|---|---|---|---|
| `StaticRandomness` (`surface.rs:792-927`) | `events`, `record_seed`, `quarantine_seed`, `on_invalid_seed`, `seed_for`, `terminal_seed_at`, `share_probe`, `signer_scheme`, `oracle_for`, `ensure_key`, `observe_epoch`, `observe_cert` + дефолты `mandatory_at`/`artifact_bytes`/`faults` | `subscribe`, `mandatory_at`, `seed`, `terminal_seed`, `can_participate`, `signer`, `oracle_for`, `ensure_key`, `observe_certificate`, `artifact_bytes`, `faults`, `observe_epoch`, `observe_cert` | ничего. Три бывших метода-приёмника были пустыми (`record_seed` и `quarantine_seed` — пустое тело, `on_invalid_seed` — `Quarantine`); теперь они стоят пустыми замыканиями в `certificate_verdict`, то есть тот же вердикт по тем же аркам. `mandatory_at` был дефолтом трейта `epoch >= DETERMINISTIC_BOOTSTRAP_EPOCH` — выписан теми же словами. `artifact_bytes`/`faults` были дефолтами `None` — выписаны как `None` |
| `Absent` (`surface.rs:1003-1073`) | тот же набор | тот же набор | ничего. `oracle_for` отвечает `None` на любой эпохе, поэтому `observe_certificate` через общий вердикт всегда возвращает `Inactive` — ровно то, что давал blanket-impl: второй `let else` по оракулу. Пустые синки под ним недостижимы, и в доке это сказано, а не сделано вид, что их нет |
| `testing::Canned` (`surface.rs:1136-1217`) | тот же набор + `mandatory_at` был ПЕРЕОПРЕДЕЛЁН (`epoch >= self.bootstrap`) | тот же набор | ничего. `record_seed`/`quarantine_seed`/`on_invalid_seed` были не пустыми — писали в настоящий `SeedStore` и спрашивали настоящий `BeaconKeys`; теперь ровно эти три операции переданы в `certificate_verdict` замыканиями, так что тест по-прежнему может утверждать, КУДА легла σ (`spy.store()`) |
| `WithholdingRandomness` (`testbed/byzantine_roles.rs:367-471`) | уже `impl Beacon` с захода А | без изменений в теле | ничего; тронут только блок `use` (пути `beacon::ceremony::…`/`dkg_msg::…`/`seed::Seed`/`wire::…` → `beacon::testing::…` и `beacon::Seed`) |

**Один общий вердикт вместо четырёх копий.** Тело `observe_certificate` (извлечение σ, гейт по
оракулу, `VerifiedSeed::check`, четыре исхода, две `error!`-строки) вынесено из blanket-impl в
свободную функцию `certificate_verdict` с пятью аргументами
`(cert, oracle_for, record, quarantine, on_invalid)` (`surface.rs:305-366`). Blanket-impl теперь — семь строк вызова
(`surface.rs:405-413`), каждая тестовая реализация — такой же вызов над своими синками.
Причина: без этого правило пришлось бы скопировать в `Canned` (у него синки не пустые), а
`Canned` — фикстура, на которой стоят тесты ingress-а `cert_inlet`; копия, разошедшаяся с
оригиналом, сделала бы эти тесты тестами копии. Тело перенесено дословно — включая обе
`error!`-строки и `unreachable!("Valid is the Ok arm")`.

## Б§2 Diff-stat

Дерево содержит А+Б; отделить их построчно нельзя (заход А не закоммичен). Файлы, которых в
статусе захода А НЕ было, содержат ТОЛЬКО правки захода Б:

~~~
$ git diff --stat -- <файлы, не тронутые заходом А>
 crates/dpos/consensus/src/byzantine.rs        |  2 +-
 crates/dpos/consensus/src/slasher/evidence.rs |  2 +-
 crates/dpos/consensus/src/sync_metrics.rs     |  2 +-
 crates/dpos/consensus/src/testbed/fakes.rs    | 11 ++++++-----
 crates/dpos/consensus/src/testbed/mod.rs      | 15 +++++++++------
 crates/dpos/consensus/src/testbed/tests.rs    |  6 ++++--
 6 files changed, 22 insertions(+), 16 deletions(-)

$ git diff --stat -- crates/dpos/consensus/src/testbed/     # А+Б
 .../dpos/consensus/src/testbed/byzantine_roles.rs  |  83 ++++++------
 crates/dpos/consensus/src/testbed/fakes.rs         |  11 +-
 crates/dpos/consensus/src/testbed/mod.rs           |  15 ++-
 crates/dpos/consensus/src/testbed/stand.rs         | 140 +++++++++++++--------
 crates/dpos/consensus/src/testbed/tests.rs         |   6 +-
 5 files changed, 150 insertions(+), 105 deletions(-)

$ git diff --stat | tail -1
 31 files changed, 2072 insertions(+), 1295 deletions(-)
~~~

Правки захода Б в файлах, которые трогал и заход А, поштучно: `testbed/stand.rs` — 2 хунка
(блок `use`: `seed::Seed` → `Seed`, `StaticRandomness` и `absent` из `beacon::testing`; и
`beacon::absent(&ctx_i)` → `absent(&ctx_i)`); `testbed/byzantine_roles.rs` — 1 хунк (блок
`use`); `beacon/mod.rs` — 4 хунка (док двери, 27 `pub(crate) mod` → `mod`, модуль `testing`,
удаление плоского тестового тира); `beacon/surface.rs` — 8 хунков (вердикт, три `impl`,
`pub(super) trait Randomness`, три дока); `executor.rs`, `epoch_manager.rs`, `cert_inlet.rs`,
`dpos.rs`, `application.rs`, `outer.rs` — только переписанные пути и, в `cert_inlet.rs`/
`dpos.rs`, четыре док-ссылки (ниже, Б§6).

## Б§3 Приватность

**Подмодули.** Все 27 объявлений в `beacon/mod.rs` — `mod x;` (было `pub(crate) mod x;`).
Счёт проверен: `git diff -- beacon/mod.rs | grep -c '^-pub(crate) mod '` → 27.
`beacon/mod.rs:45-75`.

**Продакшн-дверь после — без изменений против захода А:**

~~~rust
pub use follower::{build_follower, ArtifactFetch, FollowerInputs};
pub use plane::{build, CommitteeReads, Tasks, ValidatorInputs};
pub use seed::{constant_fallback_seed, prev_randao_from_seed, witness_fallback_seed, Seed};
pub use surface::{
    Beacon, BeaconEvent, DataFault, Observed, ObservedCertificate, PinEffort, ShareProbe,
    SignerVerdict, WithheldReason,
};
pub(crate) use dkg_engine::agreement_partition;
pub(crate) use surface::absent_unregistered;
~~~

20 `pub` + 2 `pub(crate)`, как в А§5.

**Тестовая дверь после** — `#[cfg(test)] pub(crate) mod testing` (`beacon/mod.rs:133-157`):
`DETERMINISTIC_BOOTSTRAP_EPOCH`; `decode_artifact`, `ArtifactStore`; `DkgQualFor`; `SeedStore`;
`AgreedKeyAt`, `AgreedKeys`, `BeaconKeys`, `KeySource`, `KeySources`; `BeaconMetrics`;
`encode_outcome`, `group_public_key`, `parse_outcome`, `DkgOutcome`; `Canned`; `absent`,
`for_seeds`, `BeaconResolve`, `DealtOracle`, `LiveBeacon`, `LiveBeaconConfig`,
`StaticRandomness`; `PkOracle`, `VerifiedSeed` — 25 имён. Плюс под
`#[cfg(feature = "dpos-devnet-byzantine")]`: `CommitteeFor`, `info_for`, `DealerReveal`,
`DkgBody`, `DkgMsg`, `BeaconMessage` — 6. Фича-гейт нужен: без него `cargo clippy` без фичи
даёт `unused_imports` на все шесть (проверено — сначала он его и дал).

**Проверка «внутренний путь снаружи не компилируется».** Во временно дописанной строке
`spec_exec.rs:109` (`use crate::beacon::certify::SeedStore as _ProbeSeedStore;`),
`cargo build -p fluentbase-consensus`:

~~~
error[E0603]: module `certify` is private
   --> crates/dpos/consensus/src/spec_exec.rs:109:20
    |
109 | use crate::beacon::certify::SeedStore as _ProbeSeedStore;
    |                    ^^^^^^^ private module
    |
note: the module `certify` is defined here
   --> crates/dpos/consensus/src/beacon/mod.rs:49:1
    |
 49 | mod certify;
    | ^^^^^^^^^^^^

For more information about this error, try `rustc --explain E0603`.
error: could not compile `fluentbase-consensus` (lib) due to 1 previous error
~~~

Строка убрана, файл восстановлен из копии (`tail -3` совпадает с прежним хвостом).
Заметить: это ОБЫЧНАЯ сборка, не тестовая — то есть отказ получает именно продакшн-строка,
а `#[cfg(test)]`-дверь для неё не существует вовсе.

**Тестовые модули в продакшн-файлах — что они ещё достают и через какой вход.**
Все — через `crate::beacon::testing`; КОДОВЫХ путей в `beacon::<подмодуль>` вне `beacon/`
не осталось. Оговорка к моему аудит-грепу, найденная контр-ревью (Б§8, К-1): шаблон
`beacon::(actor|artifact|carry|…)::` требует ХВОСТОВОГО `::` и потому не видит ссылок на сам
подмодуль (`[`crate::beacon::certify`]`). Таких было три, все починены; и он же не отличает
код от текста — оставшиеся семь попаданий это code-span-упоминания в доках, из них
`plane_upstream.rs:186` предсуществующий (был code-span'ом и на HEAD).

| файл | что достаёт |
|---|---|
| `executor.rs` (`#[cfg(test)]` с `:3878`) | `SeedStore`, `for_seeds`, `VerifiedSeed`, `PkOracle`, `DETERMINISTIC_BOOTSTRAP_EPOCH` |
| `epoch_manager.rs` (`:1869`) | `SeedStore`, `LiveBeacon`, `LiveBeaconConfig`, `BeaconResolve`, `BeaconKeys`, `AgreedKeys`, `KeySource`, `KeySources`, `ArtifactStore`, `BeaconMetrics`, `PkOracle`, `DETERMINISTIC_BOOTSTRAP_EPOCH` |
| `cert_inlet.rs` (`:960`) | `Canned`, `DealtOracle`, `LiveBeacon`, `LiveBeaconConfig`, `BeaconResolve`, `SeedStore`, `BeaconKeys`, `AgreedKeys`, `AgreedKeyAt`, `DkgQualFor`, `encode_outcome`, `parse_outcome`, `group_public_key`, `ArtifactStore`, `BeaconMetrics` |
| `dpos.rs` (`:4629`) | `LiveBeacon`, `LiveBeaconConfig`, `BeaconResolve`, `BeaconKeys`, `SeedStore`, `ArtifactStore`, `BeaconMetrics`, `PkOracle`, `DETERMINISTIC_BOOTSTRAP_EPOCH` |
| `application.rs` (`:1202`) | `LiveBeacon`, `LiveBeaconConfig`, `BeaconResolve`, `BeaconKeys`, `SeedStore`, `ArtifactStore`, `BeaconMetrics` |
| `outer.rs` (`:1818`) | `DETERMINISTIC_BOOTSTRAP_EPOCH` |
| `byzantine.rs` (`:32`, `#[cfg(test)] use`) | `DkgOutcome` |
| `slasher/evidence.rs` (`:642`) | `DealtOracle` |
| `testbed/stand.rs` | `StaticRandomness`, `absent`, `CommitteeFor` (под фичей) |
| `testbed/tests.rs` | `decode_artifact`, `group_public_key` |
| `testbed/byzantine_roles.rs` | `info_for`, `DkgMsg`, `DkgBody`, `DealerReveal`, `BeaconMessage` |
| `testbed/fakes.rs` | ничего: перешёл на публичные `beacon::Seed` и `beacon::prev_randao_from_seed` |

**`Randomness` сузился до `pub(super)`** (`surface.rs:443`). Это не косметика: компилятор
проверил, что вне `beacon/` его никто не называет — сборка зелёная и без фичи, и с ней.
Из-за этого §5.1-й тезис «шов подстановки — трейт» стал проверяемым, а не заявленным.

## Б§4 Ворота

~~~
$ cargo test -p fluentbase-consensus
running 636 tests
test result: ok. 636 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 20.71s
running 3 tests
test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
running 6 tests
test result: ok. 5 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.01s
running 13 tests
test result: ok. 13 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s
running 1 test
test result: ok. 0 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.00s

$ cargo test -p fluentbase-consensus --lib testbed::
running 27 tests
test result: ok. 27 passed; 0 failed; 0 ignored; 0 measured; 609 filtered out; finished in 20.14s

$ cargo test -p fluentbase-consensus --features dpos-devnet-byzantine testbed::
running 35 tests
test result: ok. 35 passed; 0 failed; 0 ignored; 0 measured; 609 filtered out; finished in 31.32s

$ cargo test -p fluentbase-node
running 57 tests
test result: ok. 57 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 57.38s

$ cargo clippy -p fluentbase-consensus -p fluentbase-node --all-targets
warning: large size difference between variants
warning: `fluentbase-node` (lib test) generated 1 warning
warning: `fluentbase-node` (lib) generated 1 warning (1 duplicate)

$ cargo clippy -p fluentbase-consensus --all-targets --features dpos-devnet-byzantine
(0 строк warning/error)

$ cargo fmt -p fluentbase-consensus -p fluentbase-node -- --check
(0 строк `Diff in`)
~~~

Единственное предупреждение — `large_enum_variant` на `ValidatorUpstream`
(`crates/node/src/dpos.rs:1958`), то же, что в А§6: файл в диффе есть, но эта строка — не моя,
энум заходом Б не тронут.

~~~
$ git status --short | grep -v '^!!'
 M .dpos-study/PLAN.md
 M crates/dpos/consensus/src/application.rs
 M crates/dpos/consensus/src/beacon/actor.rs
 M crates/dpos/consensus/src/beacon/carry.rs
 M crates/dpos/consensus/src/beacon/certify.rs
 M crates/dpos/consensus/src/beacon/follower.rs
 M crates/dpos/consensus/src/beacon/keys.rs
 M crates/dpos/consensus/src/beacon/metrics.rs
 M crates/dpos/consensus/src/beacon/mod.rs
 M crates/dpos/consensus/src/beacon/plane.rs
 M crates/dpos/consensus/src/beacon/resolve.rs
 M crates/dpos/consensus/src/beacon/surface.rs
 M crates/dpos/consensus/src/byzantine.rs
 M crates/dpos/consensus/src/cert_inlet.rs
 M crates/dpos/consensus/src/dpos.rs
 M crates/dpos/consensus/src/engine.rs
 M crates/dpos/consensus/src/epoch_manager.rs
 M crates/dpos/consensus/src/executor.rs
 M crates/dpos/consensus/src/outer.rs
 M crates/dpos/consensus/src/scheme.rs
 M crates/dpos/consensus/src/slasher/evidence.rs
 M crates/dpos/consensus/src/spec_exec.rs
 M crates/dpos/consensus/src/sync_metrics.rs
 M crates/dpos/consensus/src/testbed/byzantine_roles.rs
 M crates/dpos/consensus/src/testbed/fakes.rs
 M crates/dpos/consensus/src/testbed/mod.rs
 M crates/dpos/consensus/src/testbed/stand.rs
 M crates/dpos/consensus/src/testbed/tests.rs
 M crates/node/src/cert_inlet.rs
 M crates/node/src/consensus_rpc/state.rs
 M crates/node/src/dpos.rs
~~~

Шесть файлов сверх списка захода А, все — следствие приватности:
`byzantine.rs`, `slasher/evidence.rs`, `sync_metrics.rs`, `testbed/{fakes,mod,tests}.rs`.
В `byzantine.rs`, `slasher/evidence.rs` и `sync_metrics.rs` — по одной строке
(`#[cfg(test)] use` / путь в фикстуре / путь в док-комментарии).

Про разрешённый список файлов: правки в `cert_inlet.rs` и `dpos.rs` вне их
`#[cfg(test)]`-блоков есть — ПЯТЬ док-ссылок (`cert_inlet.rs:322,327,406`, `dpos.rs:102,122`),
шестая — `sync_metrics.rs:6`; разбор в Б§6. Три из шести сломала МОЯ пакетная замена путей,
остальные указывали в подмодули, которые заход Б сделал приватными. После контр-ревью
добавилось ещё шесть правок в доках — Б§8.

## Б§5 Логические коммиты

| путь | группа | заголовок |
|---|---|---|
| `beacon/surface.rs` | 1 | `refactor(beacon): put every beacon implementation on the trait and give them one certificate verdict` |
| `beacon/mod.rs`, `consensus/src/{executor,epoch_manager,cert_inlet,dpos,application,outer,byzantine,sync_metrics}.rs`, `slasher/evidence.rs`, `testbed/*.rs` | 2 | `refactor(beacon): make every submodule private and give the crate's tests one door` |
| `.claude/dpos_architecture/{00_preamble,15_…}.md`, `.dpos-study/*` | 3 | `docs(dpos): record the beacon boundary's second pass` |

Оговорка: группу 2 разбить нельзя. Как только `pub(crate) mod` становится `mod`, все
одиннадцать файлов-потребителей перестают собираться до тех пор, пока их пути не переписаны;
это одна атомарная правка, а не набор. Группа 1 собирается сама по себе (проверено:
`cargo test --lib --no-run` был зелёным до правки видимости). Группы 1 и 2 обе стоят ПОСЛЕ
всех пяти групп захода А (А§8).

## Б§6 Что всплыло для 5.1-5.4

- **`09_followers.md:216-221` противоречит коду.** Там таблица, чей заголовок называет
  `impl Randomness` и колонку `seed_for`, и строка
  «`StaticRandomness` (`#[cfg(test)]` на структуре и на impl)». После захода Б `StaticRandomness`
  реализует `Beacon`, а не `Randomness`, и метода `seed_for` у него нет.
  **[ИСПРАВЛЕНО в тексте, заход В (F-18)]** Утверждение «Файл 09 не входит в список
  документов этой задачи, поэтому НЕ тронут» было неверно уже когда писалось: заход А
  поставил в нём маркеры `[REVISED 2026-09-11, PLAN row 5.0 pass A]` на `:208` и `:246`.
  Заход В тронул его снова и починил саму таблицу: в маркере над ней теперь сказано, что
  колонку `seed_for` для трёх фикстур надо читать как `Beacon::seed`, потому что
  `Randomness` после Б говорят только `LiveBeacon` и `FollowerRandomness`. [KNOWN]
- **Четыре продакшн-док-ссылки указывали в подмодули, ставшие приватными.** Починены как
  обычные code-span, без ссылки: `sync_metrics.rs:6` (`BeaconMetrics`), `cert_inlet.rs:322`
  (`CommitteeFor`), `:327` (`DkgActor`), `:406` (`AgreedKeys`), `dpos.rs:102` (`SeedStore`),
  `:122` (`ArtifactStore`) — шесть, не четыре. Три из них сначала сломала моя же пакетная
  замена (она увела их в `beacon::testing`, то есть продакшн-док стал ссылаться на
  `#[cfg(test)]`-дверь); отловлено грепом `beacon::testing::` по строкам-комментариям.
- **Для 5.1.** `for_keys` теперь не покидает `beacon/` вообще (единственные потребители — два
  теста в `surface.rs`), так что 5.1 может удалить его, не тронув ни одного файла снаружи.
  `for_seeds` — иначе: его держит фикстура `executor.rs:4561,4590`, и его удаление в 5.2
  потребует дать executor-у другой вход к `LiveBeacon` над одним `SeedStore`.
- **Для 5.1-5.4 в целом.** `beacon::testing` — теперь явный счёт того, что рядам ещё надо
  держать называемым: 25 имён + 6 под фичей. Каждая строка, которую 5.1/5.2 удаляет из
  внутренностей, обязана исчезнуть и оттуда; если после ряда список не укоротился, ряд
  что-то оставил.
- **Вне рамки, одной строкой.** `Canned` живёт в `surface::testing`, а дверь называется
  `beacon::testing` — два модуля с одним именем на разных уровнях. Путаницы компилятор не
  допускает, но при удалении `surface.rs` в 5.1 имя стоит развести.

## Б§7 Слабые места

1. **Отделить дифф захода Б от захода А построчно нельзя** — А не закоммичен. Всё, что в Б§2
   названо «правки Б», я перечислил по своим хункам, а не вывел командой; для шести файлов,
   которых в статусе А не было, это проверяется `git diff` и там точно.
2. **`certificate_verdict` перенесён дословно, но не защищён тестом на дословность.** Что
   исходы совпали, доказывают только прежние тесты ingress-а (`cert_inlet`, 636 в lib).
   Если бы я переставил, скажем, `local`-арм и `on_invalid`-арм местами для σ, пришедшей с
   провода, красный дал бы только `cert_inlet`-тест с `RefuseLoud`; арм `RefuseQuiet`
   тестом не покрыт ни до, ни после.
3. **`Absent::observe_certificate` через общий вердикт — путь, который не исполняется дальше
   второго `let else`.** Что синки под ним пустые, ничем не проверяется: любой из них можно
   заменить на панику, и все тесты останутся зелёными.
4. **Фича-гейт на шести re-export-ах проверен только двумя конфигурациями сборки** (с фичей
   и без). Третьей конфигурации (`--no-default-features`) я не запускал — и это слабое место
   оказалось ПУСТЫМ: контр-ревью её прогнало (зелёная), но фича `std` в
   `crates/dpos/consensus/Cargo.toml:90` пуста, а `dpos-devnet-byzantine` в default не входит,
   так что конфигурация совпадает с default. Проверять там было нечего.
5. **Приватность проверена ОДНИМ зондом** (`certify::SeedStore` из `spec_exec.rs`). Что
   каждый из 27 подмодулей закрыт, я вывел из `mod x;` в одном файле, а не из 27 зондов.
   ЗАКРЫТО контр-ревью: 27 зондов, по одному на подмодуль, плюс обратный зонд из продакшна
   в `beacon::testing` — ни один не прошёл (Б§8, находка К-4).
6. **`testbed/mod.rs` и `15_…md §15.a` я переписал по своему чтению кода, а не сверял с
   прежним текстом построчно** — в частности утверждение «`WithholdingRandomness` делегирует
   двенадцать из тринадцати операций» я посчитал по телу `impl Beacon`
   (`byzantine_roles.rs:367-471`), а не по diff-у.

**Команда чтения длинных строк.** `awk 'length($0)>500 {print FILENAME":"NR": "length($0)}'`
для поиска, `sed -n 'Np' file | fold -w 160` для чтения; `cut -c1-N` применялся только к
выводу `grep`, не к чтению файла.

**Счётчики текстовых проверок** на всей части Б, включая Б§8 — те же пять, что в А§9, после
правок: строк, оканчивающихся на многоточие — 0; строк с нечётным числом обратных кавычек
вне блоков кода — 0 (пять раз попадался код-спан, перенесённый через конец строки:
переформулировано, чтобы спан помещался в строку); удвоенных запятых — 0; пустых пар
обратных кавычек — 0; пустых круглых скобок вне блоков кода и вне инлайн-спанов — 0
(единственные `()` — `spy.store()` и две самоссылки, все внутри спанов). Считано скриптом на
python по диапазону от заголовка части Б до конца файла.

## Б§8 Контр-ревью захода Б

Три ревьюера в свежем контексте (Opus 5), каждый со своей областью и с Bash — в отличие от
захода А, где у агентов типа `reviewer` Bash-а не было и сверку с базой никто сделать не смог
(А§10). Области: семантика переписанных фикстур; граница и приватность; фактчек этой части
журнала. Всем троим запрещено порождать подагентов и править дерево.

**Ни одной находки в поведении кода.** Первый прогнал все 13 методов × 3 фикстуры против
`git show HEAD:` и не нашёл входа, на котором фикстура отвечает иначе. Второй не нашёл ни
одной утечки внутреннего типа в сигнатуры всех 20 публичных имён двери и ни одного мёртвого
имени в `beacon::testing`. Третий подтвердил «ни одно утверждение теста не тронуто» машинно:
шесть изменённых строк с `assert*`, все — переименования вызова, и счёт `#[test]` по каждому
файлу совпадает с базой (executor 118, cert_inlet 28, epoch_manager 13, dpos 28,
application 31, outer 5, surface 14, testbed/tests 35, byzantine 2, slasher/evidence 10).

**Что починено после ревью.**

| # | находка | правка |
|---|---|---|
| К-1 | Мой аудит-греп требовал ХВОСТОВОГО `::` и не видел ссылок на сам подмодуль. Их было три, все живые intra-doc: `engine.rs:44` `[`crate::beacon::certify`]`, `epoch_manager.rs:429` `[`crate::beacon::dkg_engine`]`, `epoch_manager.rs:1311` `[`crate::beacon::keys`]` | переведены в code-span; повторный греп без хвостового `::` пуст |
| К-2 | `beacon/mod.rs:42` — продакшн-док САМОЙ двери ссылался на `[`testing`]`, которого без `cfg(test)` нет: `warning: unresolved link to 'testing'`. Ровно тот класс, что я ловил у других | ссылка снята, оставлен code-span и одна строка о том, ПОЧЕМУ здесь нельзя ссылку |
| К-3 | `epoch_manager.rs:195` — ссылка на `Randomness` перестала резолвиться: заход А снял импорт, ссылку не переписал. Прямо против моего «вне `beacon/` не называется» | заменена на code-span `Beacon` |
| К-4 | `beacon/plane.rs:492` — `[`WithheldReason::GeometryUnfrozen`]` не резолвится (тип в `plane.rs` не импортирован). Ссылка и сам док — заход А | полный путь `(super::WithheldReason::GeometryUnfrozen)` |
| К-5 | Крейт-видимых элементов двери 23, а не 22: я не считал `pub(crate) const JOURNAL_RETENTION_EPOCHS` (`beacon/mod.rs:103`), у которого вне `beacon/` НОЛЬ потребителей | сделан приватным. Дочерний модуль видит приватные элементы родителя, так что `pub(crate)` тут ничего не давал, кроме лишней ширины |
| К-6 | Пять фактических ошибок в этой части журнала: «24 подмодуля» вместо 27; «греп пуст» вместо «пуст по кодовым путям»; «четыре док-ссылки» вместо пяти в двух файлах и шести всего; сокращения в Б§0 п.4, приписывающие `epoch_manager` имена `for_seeds`/`VerifiedSeed`, которых он не берёт; `surface.rs:1136-1218` вместо `-1217` | исправлены на месте |

После правок `cargo doc -p fluentbase-consensus --no-deps` даёт 9 `unresolved link`, и ни
одной про beacon-границу: `[`E`]` ×4 (в том числе предсуществующая `beacon/mod.rs:2`),
`[`subscribe`]` (`dpos.rs:1023`, есть и на HEAD), `Actor::enter` ×2, `FakeMarshal`,
`Scheme::verify_attestation`. [KNOWN]

**Что ревью показало сильнее, чем я.** Зонд `pub(crate) use surface::Randomness as _ProbeRnd;`
в самой двери даёт `error[E0365]: 'Randomness' is private, and cannot be re-exported` — то
есть дверь физически НЕ МОЖЕТ его расширить, не тронув объявление в `surface.rs`. Это
свойство, а не соблюдение соглашения, и оно сильнее моей формулировки «вне `beacon/` никто не
называет».

**Что ревью оставило открытым (не чинил, называю).**

- `crates/node/src/dpos.rs:526,542,884` — три комментария всё ещё говорят `Arc<dyn Randomness>`,
  тогда как тип называется `Arc<dyn Beacon>`. Заход А; файл не в списке этой задачи.
- Заход А утверждал (А§6), что фичевый clippy на узле не собирается
  (`E0063` на `node/dpos.rs:2176`), и потому фичевые
  ворота ставились только на консенсус. Проверено в этом заходе: **собирается и чист**
  (одно предсуществующее `large_enum_variant`). Утверждение А устарело. [KNOWN]
- `spec_exec.rs:109` в блоке зонда (Б§3) фактчекер счёл невозможным, потому что в файле 106
  строк. Номер ВЕРЕН: `printf` дописывал пустую строку (107) и комментарий (108) перед
  `use` (109). Проверять не стал бы, если бы не отчёт — оставляю разбор здесь.
- Три наблюдения о заходе А, не о Б, вынесены как есть: из локальной `error!` вердикта
  пропало поле `?check`, которое было на `HEAD:spec_exec.rs:79-84`; `local` в
  `certificate_verdict` выводится из ТИПА сертификата (`Notarization ⇒ true`), а не из
  происхождения σ — сегодня верно, потому что единственный конструктор `Notarization`
  локальный, но чужая нотаризация с провода была бы отвергнута как «locally recovered»
  вместо карантина; возвращаемый `Observed` не читается ни на одном продакшн-сайте — все три
  `let _ =` при `#[must_use]` на типе.

# Часть В — строка 5.0, заход В: 25 находок ревью `history/E5-0-REVIEW.md`

База та же, `1e5f394e`. Объект — то же незакоммиченное дерево (30 файлов под `crates/`),
плюс `.claude/dpos_architecture/{00_preamble,09_followers,15_smoke_cases_…}.md` и одна
строка `.dpos-study/PLAN.md:107`. Хунк Э4 в `PLAN.md` — чужая незакоммиченная работа, не
тронут (проверено: `git diff .dpos-study/PLAN.md` показывает в нём те же строки, что и до
захода).

## В§0 Прямые ответы

**1. FIXED 17 · RECORDED 7 · REJECTED 1 (частичный).**
FIXED: F-1, F-2, F-3, F-4, F-5, F-6, F-7, F-9, F-11, F-13, F-14, F-15, F-16, F-17, F-19,
F-23, F-24. RECORDED: F-8, F-10, F-12, F-18, F-20, F-22, F-25. F-21 закрыт смешанно:
(a)-(e) RECORDED строками Д-9…Д-13, (f) **REJECTED** — `Observed`, `DataFault` и
`WithheldReason` стоят в сигнатурах самой двери и снять их экспорт нельзя:
`Observed` — тип возврата `Beacon::observe_certificate` (`beacon/surface.rs:175`),
`DataFault` — тип возврата `Beacon::faults` (`:218`), `WithheldReason` — поле
экспортированного `ShareProbe::Withheld` (`:77`). Приватный тип в публичной сигнатуре не
компилируется чисто, так что «нет внешнего потребителя» здесь не аргумент за удаление —
их потребитель структурный, а не текстовый. [KNOWN]

**2. Ворота — verbatim, по одной строке.**

- `cargo test -p fluentbase-consensus --lib` → `test result: ok. 640 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 20.59s`
- `cargo test -p fluentbase-consensus --lib --features dpos-devnet-byzantine testbed::` → `test result: ok. 35 passed; 0 failed; 0 ignored; 0 measured; 613 filtered out; finished in 31.40s`
- `cargo test -p fluentbase-node --lib` → `test result: ok. 59 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 60.87s`
- `cargo clippy -p fluentbase-consensus -p fluentbase-node --all-targets` → ровно три строки: `warning: large size difference between variants`, `warning: `fluentbase-node` (lib) generated 1 warning`, `warning: `fluentbase-node` (lib test) generated 1 warning (1 duplicate)`
- `cargo clippy -p fluentbase-consensus --all-targets --features dpos-devnet-byzantine` → ноль строк `warning`/`error`
- `cargo fmt --check` → ноль строк `Diff in` (три места переформатировал `cargo fmt`: сигнатура `classify_seed_wake`, его первый арм и один `matches!` в новом тесте)
- `cargo doc -p fluentbase-consensus --no-deps 2>&1 | grep -c "unresolved link"` → `9`

Расхождение с 636/35/57: **+4 теста lib консенсуса и +2 теста узла, все новые, ни одного
удалённого.** 636 → 640: `executor::tests::a_closed_beacon_channel_disarms_the_wake_up_arm_instead_of_spinning`,
`executor::tests::the_wake_up_arm_ignores_the_other_two_event_classes_and_derives_on_lagged`,
`epoch_manager::tests::the_seed_class_never_leaves_the_reconcile_wake`,
`epoch_manager::tests::an_overflowed_stream_reaches_the_reconcile_arm`. 57 → 59:
`dpos::tests::the_qual_cursor_refuses_the_window_where_the_committee_cursor_falls_back_to_genesis`,
`dpos::tests::with_a_finalized_marker_the_two_legs_read_at_one_cursor`. Стенд 35 — без
изменений. Девять `unresolved link` — тот же список, что в Б§8, ни одной про границу
beacon-а: `E` ×4 (`beacon/mod.rs:2`, `cert_inlet.rs:345`, `cold_start_jump.rs:795`,
`executor.rs:539`), `subscribe` (`dpos.rs:1023`), `Actor::enter` ×2 (`engine.rs:142`,
`scheme.rs:47`), `FakeMarshal` (`executor.rs:371`), `Scheme::verify_attestation`
(`slasher/evidence.rs:510`). [KNOWN]

**3. F-8: да, нотаризация с провода доходит до `certificate_verdict` с `local = true`.**
Путь: сертификат приходит в `Message::Verified(Certificate::Notarization(..), from_resolver)`
(`simplex/actors/voter/actor.rs:996-1007`) → `handle_notarization` кладёт его в состояние
раунда → `notify` зовёт `try_broadcast_notarization` (`:671-672`) → `state.broadcast_notarization(view)`
(`voter/state.rs:395-399`) → `Round::broadcast_notarization` отдаёт ХРАНИМЫЙ сертификат
независимо от того, как он туда попал (`voter/round.rs:438-447`; собственный тест
commonware `broadcast_notarization_without_local_notarize`, `:743-766`, строит его «entirely
from remote votes») → `reporter.report(Activity::Notarization(..))` (`voter/actor.rs:528-531`).
Второй путь — реплей журнала (`:749-756`).
**Предписанное лекарство («`local` — явный параметр от вызывающего») ОТКЛОНЕНО, и вот
опровергающий якорь:** `Activity::Notarization` не несёт поля происхождения, и все три
`report`-сайта отдают один и тот же вариант, так что единственный вызывающий
(`spec_exec.rs:92`) мог бы передать только константу — то же самое утверждение, перенесённое
из одного места, где оно видно, в место, где его нельзя опровергнуть. Сделано другое:
поведение оставлено ровно как на HEAD, а флаг и доки перестали называть его фактом о
происхождении — он теперь `speculation` и называет ДВЕРЬ (`surface.rs:331-334`, `:358`),
а перечень проводных путей с якорями в commonware записан на самом типе
(`surface.rs:221-244`). Это RECORDED, а не FIXED: дерево изменилось только в именах и доках.
[KNOWN]

**4. F-10: да, две поверхности reth — это ОДИН объект, и вторая всегда не хуже первой.**
`BlockIdReader::finalized_block_number` — производная от `finalized_block_num_hash`
(`storage-api/src/block_id.rs:121-123`), а у `BlockchainProvider` та реализована ровно как
`Ok(self.canonical_in_memory_state.get_finalized_num_hash())`
(`provider/src/providers/blockchain_provider.rs:279-281` → `chain-state/src/in_memory.rs:422-424`).
То есть HEAD брал НОМЕР из того же трекера и потом разрешал хэш вторым запросом
(`block_hash(n)` → `consistent.rs:577-583`, «в памяти, иначе БД»), а дерево берёт хэш из той
же пары. Отличаться они могут только если канонический хэш на высоте `n` не тот, что у
финализированного заголовка — а финализированный ставится только заголовком, найденным
каноническим (`engine/tree/src/tree/mod.rs:3094-3106, 3117-3131`), и реорг ниже финализации
исключён. Что ВТОРОЙ запрос может добавить — это `None` там, где трекер отвечает `Some`.
Сделано: ничего не откатывал, источник дерева оставлен; дельта и якоря записаны здесь
(RECORDED). Утверждение А§3 «тот же хэш» теперь имеет доказательство, которого у него не
было. [KNOWN]

**5. F-2/F-3: рёберная семантика HEAD восстановлена, кроме одного пункта, который журнал
уже называл своим.** Сброс `catchup_no_progress` снова происходит ТОЛЬКО на классе
участия (`epoch_manager.rs:862-880`), как на `HEAD:epoch_manager.rs:764-770`; класс ключа
его не трогает, как `HEAD:epoch_manager.rs:816-822`. Пробуждение sweep-а на классе участия
ОСТАВЛЕНО — это изменение А§3 назвал прямо («share теперь будит sweep»), и оно же снимает
требование «два плеча обязаны совпадать вручную». Круглосуточный такт снят без второго
канала: класс σ отбрасывается ВНУТРИ future (`epoch_manager.rs:394-405`,
`next_reconcile_wake`), так что плечо больше не завершается раз в раунд и цикл не
пересобирает два `notified()` на каждый блок. Остаточная честная оговорка: задача всё
равно ПОЛЛИТСЯ на каждый `SeedRecorded` — общий broadcast будит всех подписчиков; чего
больше нет, так это полного оборота `select!`. [KNOWN]

**6. Имена, покидающие `beacon/`, после захода В: 22 продакшн-имени (20 `pub` + 2
`pub(crate)`) плюс тестовая дверь из 25 имён (+6 под фичей) — ровно столько же, сколько
после захода Б; заход В не добавил и не убрал ни одного.** Покрыты строками Д: шесть
отклонений F-21 закрыты пятью новыми строками Д-9…Д-13 (по одной на (a), (b), (c), (d), (e))
плюс REJECTED на (f), который не отклонение, а структурная необходимость (В§0.1). Плюс
Д-14 — новое отклонение ЭТОГО захода: у `CommitteeReads` теперь ДВА курсора, а §5.1 просила
один. Имён вне обоих списков — **ноль**. [KNOWN]

**7. Новые тесты — шесть, красным до правки был один.**
- `executor::tests::a_closed_beacon_channel_disarms_the_wake_up_arm_instead_of_spinning`
  (`executor.rs:12287`) — пинует F-4: закрывает сендер, ПРОВЕРЯЕТ саму посылку спина
  (receiver готов дважды подряд без await) и требует `SeedWake::Disarm`. **Был бы красным до
  правки:** до неё `classify_seed_wake` не существовал, а арм на `Closed` проваливался в
  `try_eager_finalized_derive`; после подстановки старой ветки (`Closed` → derive) тест
  падает на первом же `assert_eq!(…, SeedWake::Disarm)`.
- `executor::tests::the_wake_up_arm_ignores_the_other_two_event_classes_and_derives_on_lagged`
  (`:12319`) — что `Lagged` остаётся пробуждением, а два чужих класса — нет.
- `epoch_manager::tests::the_seed_class_never_leaves_the_reconcile_wake` (`epoch_manager.rs:3285`)
  — F-3: три подряд `SeedRecorded` проглатываются внутри future, `KeyAvailable` выходит,
  `Closed` НЕ проглатывается.
- `epoch_manager::tests::an_overflowed_stream_reaches_the_reconcile_arm` (`:3318`) — `Lagged`
  доходит до плеча (иначе оба эффекта потерялись бы молча).
- `dpos::tests::the_qual_cursor_refuses_the_window_where_the_committee_cursor_falls_back_to_genesis`
  (`node/dpos.rs:2502`) — F-11: в окне `fin == None && live > 0` комитетный курсор отвечает
  `Some((live, 0))`, а курсор `dkgQual` — `None`.
- `dpos::tests::with_a_finalized_marker_the_two_legs_read_at_one_cursor` (`:2522`) — что
  разделение курсоров не разошлось с Д-7 нигде, кроме этого окна.
Плюс два переписанных: оба теста «без потери пробуждения» в `certify.rs` переведены с
`notify.notified()` на `events().subscribe()` (`certify.rs:568-651`), и тест дренажа узла
(`node/dpos.rs:2615`) сменил фиксированный `sleep(50 ms)` на наблюдаемый факт «писатель
уже прочитал запись» + опрос хэндла (F-24). [KNOWN]

**8. Где ревью было неправо или преувеличило.**
(а) **F-8, предписание.** Находка верна (проводной путь есть), лекарство — нет: явного
параметра неоткуда взять, `Activity::Notarization` не несёт происхождения (В§0.3). Ревью
подало «сделать параметром» как выполнимое; выполнимо оно только как константа на сайте.
(б) **F-21(f).** «Три экспортированных типа без внешнего потребителя — снять экспорт» —
снять нельзя, все три стоят в сигнатурах двери (В§0.1). Ревью посчитало текстовых
потребителей и не посчитало структурных.
(в) **F-11, формулировка «на HEAD чтения не было вовсе».** Верно для валидаторского
`dkg_qual_at`, но ревью не отметило, что §5.1 проекта ЯВНО требует читать бит на
`max(fin, live)` (`E5-BEACON-DESIGN.md:562`, и строка 5.1 в §7). То есть слияние курсоров —
не самовольство захода А, а выполнение спеки; дефект был у`отката`, а не у слияния, и
починка это и отражает (окно `fin == None` закрыто, `max(fin, live)` сохранён).
(г) **F-22, «паника заменила лог-строку на продакшн-пути».** Наполовину: на
ФИНАЛИЗАЦИОННОЙ двери `unreachable!("Valid is the Ok arm")` стоял уже на HEAD
(`HEAD:cert_inlet.rs:3040`), слово в слово. Изменился один путь из двух, а не «продакшн-путь».
(д) **F-13, список из семи строк.** Их восемь: ревью не назвало
`beacon/keys.rs:547` (`crate::beacon::follower::for_follower` в интра-док-ссылке).
Исправлены все восемь, кроме `cert_follow.rs:107` — файл вне списка правимых в этом заходе.
(е) **F-24, «поллить хэндл».** Одного поллинга хэндла мало: он говорит «ещё не завершился»,
но не «задача вообще запускалась», а именно это фиксированный сон и покупал. Сделано
сильнее сна и сильнее предложенного: сначала наблюдаемый факт, что писатель ПРОЧИТАЛ
запись, потом `Poll::Pending` на хэндле.
[KNOWN]

**9. Ответ «5.0 готов к коммиту» не изменился — да.** Все ворота зелёные, блокеров
по-прежнему нет, и то, что ревью называло блокером до коммита (мёртвые `Notify` с живой
документацией, осиротевший док, неназванные дельты, док-дрейф, самопротиворечивая строка
PLAN), закрыто. Раскладка по коммитам — В§4: заход В не создаёт новой группы, он дописывает
в те же шесть групп захода А и три группы захода Б, кроме одной новой строки в группе
документации. [KNOWN]

## В§1 Таблица прослеживания

| id | вердикт | file:line дерева после правки | что сделано | что будет, если применить наполовину |
|---|---|---|---|---|
| F-1 | FIXED | `beacon/certify.rs:237`, `:44-57`, `:92-104`, `:180-186`; `beacon/keys.rs:38-42`, `:149-152`, `:272-280`, `:379-380` | Поля `notify` и вызовы `notify_one` удалены из `SeedStore` и `BeaconKeys`; доки про «арм executor-а» и «waiter population of ONE» переписаны на `broadcast`/per-consumer notifier; четыре теста переведены на `events().subscribe()` и `store.subscribe()` | Убрать поля, оставив доки: 5.1/5.2 прочитают «правило одного waiter-а» как живой контракт и будут его защищать. Убрать доки, оставив поля: мёртвый механизм переживёт ещё один заход и его снова придётся доказывать |
| F-2 | FIXED | `epoch_manager.rs:862-880` | Сброс `catchup_no_progress` вернулся на класс участия и ТОЛЬКО на него (`Err(_)` тоже, потому что потерянный класс мог быть любым); класс ключа его не трогает, как на HEAD | Вернуть сброс, не тронув пробуждение (F-3) — цикл продолжит крутиться раз в раунд и «рёберная семантика восстановлена» станет неправдой |
| F-3 | FIXED | `epoch_manager.rs:394-405` (`next_reconcile_wake`), `:859` | Класс σ отбрасывается ВНУТРИ future, а не `continue`-ом в теле, так что плечо не завершается раз в раунд и два `notified()` не пересобираются на каждый блок. `Lagged`/`Closed` возвращаются наружу | Отфильтровать в мосте `plane.rs` вместо этого — executor перестанет получать свой единственный класс. Вернуть `continue` — вернётся такт |
| F-4 | FIXED | `executor.rs:196-224` (`SeedWake`, `classify_seed_wake`), `:1215`, `:1532`, `:1538-1550` | `Err(Closed)` разоружает арм флагом в гвардии и пишет один `error!`; классификация вынесена из `select!` и покрыта тестом, который сам доказывает посылку спина | Разоружить без `error!` — узел молча теряет освобождение удержанного tip-а. Залогировать без разоружения — горячий цикл остаётся |
| F-5 | FIXED | `beacon/plane.rs:203-215` | `SupervisedChildren` строится ДО `spawn` и въезжает в future захватом, так что дроп-гвардия существует с момента существования future — и срабатывает даже на ветке `aborted` внутри самого `spawn`, где замыкание не зовётся (`runtime/src/tokio/runtime.rs:575-578`) | Оставить конструктор внутри async-блока: `Handle` без `Drop` (`runtime/src/utils/handle.rs:106-117`), и abort до первого poll-а оставит шесть детей живыми |
| F-6 | FIXED | `cert_inlet.rs:3001-3004` (короткий док типа), `:3060-3077` (содержательный — на `spawn_finalized`) | Осиротевший блок снят с `UpstreamResolver`; его содержание (две двери, кто из них прунит) перенесено туда, где оно про код, а не рядом с ним | Просто удалить текст — исчезнет единственная запись о том, почему σ через дверь-пробел остаётся непрунутой |
| F-7 | FIXED | `spec_exec.rs:92-98`, `cert_inlet.rs:860-864`, `:3129-3135` | Все три сайта единообразно `let _observed = …` с одной строкой «PLAN 5.2 даёт ему читателя»; `#[must_use]` оставлен | Снять `#[must_use]` вместо этого — тип перестанет требовать читателя и в 5.2, когда читатель как раз появится |
| F-9 | FIXED | `beacon/surface.rs:358-369` | Поле `?check` вернулось в локальную `error!` через `Err(check @ SeedCheck::Invalid)`; текст строки оставлен байт-в-байт с `HEAD:spec_exec.rs:79-84` | Вернуть поле, поменяв текст — сломается grep по соакам, ради которого строка и опознаваема |
| F-10 | RECORDED | `consensus/dpos.rs:3431-3433` | Источник дерева оставлен; доказано по reth, что это ТОТ ЖЕ трекер, что и у HEAD, и что второй запрос HEAD-а мог только не найти хэш (В§0.4) | Откатить на HEAD «для надёжности» — вернётся второй запрос, который на follower-е может ответить `None` там, где трекер знает ответ |
| F-11 | FIXED | `beacon/plane.rs:121-134` (`qual_read_at`), `node/dpos.rs:1082-1110` (`committee_cursor`/`qual_cursor`), `:1439-1451`, `consensus/dpos.rs:3440-3442`, `testbed/stand.rs:1860-1870` | У `dkgQual` свой курсор: `fin.is_some()` обязателен, отката на `block_hash(0)` нет; `max(fin, live)` сохранён, так что Д-7 не нарушен. Тест на окно | Закрыть окно только в валидаторе — follower останется на своей ветке и правило разъедется по узловым классам (ровно то, что Д-7 сводил) |
| F-12 | RECORDED | `node/dpos.rs:526-548`, `:886-897` | Порядок дропа НЕ менялся; комментарии переписаны под ТРИ писателя, с поимённым перечнем всех четырёх держателей `Arc<dyn Beacon>` и с тем, какой из них чем освобождается. «Все клоны умирают до дренажа» остаётся [ГИПОТЕЗА] по ВРЕМЕНИ (abort дропает future асинхронно) при [KNOWN] по достижимости | Переписать комментарий, не перечислив держателей, — следующая правка снова не будет знать, какой список поддерживать |
| F-13 | FIXED | `node/dpos.rs:377`, `:917`, `:526-548`, `:886-897`; `consensus/dpos.rs:3187`; `beacon/keys.rs:547` | Восемь устаревших имён (не семь — ревью пропустило `keys.rs:547`) приведены к дереву: `for_follower` → `build_follower`, `Arc<dyn Randomness>` → `Arc<dyn Beacon>`, «BOTH/two» → «три писателя». ОСТАЛОСЬ: `cert_follow.rs:107` — файл вне списка правимых в этом заходе | Поправить часть — grep по `for_follower` перестанет находить остаток, и он переживёт 5.1 |
| F-14 | FIXED | `beacon/mod.rs:27-41`, `:43` | «три staking reads» → две названные поставки с ролями каждой; «On the list and deliberately NOT public» → «On NEITHER list»; добавлен абзац, почему четыре имени словаря экспортированы без внешнего читателя | Починить счёт, не починив «On the list» — фраза продолжит читаться как противоположность смыслу |
| F-15 | FIXED | `15_smoke_cases_…md:381-383` | «forwards fourteen» → «forwards the other twelve», с якорем `byzantine_roles.rs:367-471` и маркером `[REVISED 2026-09-11, PLAN row 5.0 pass В]` | Поправить одно из двух мест — файл останется противоречащим сам себе |
| F-16 | FIXED | `09_followers.md:206-213` | Список из пяти `impl Beacon` перепинен по дереву (`surface.rs:400/824/1035/1168`, `byzantine_roles.rs:367`) и прямо сказано, что в `follower.rs` его нет, а есть `impl Randomness for FollowerRandomness` (`:451`) | Перепинить якоря, не исправив состав, — список так и будет называть файл без `impl Beacon` и молчать про роли |
| F-17 | FIXED | `09_followers.md:236-251` | Все семь якорей перепинены, блок получил маркер `[REVISED … pass В]`; заодно снято уже неверное «Its own doc comment is now stale» — этот док в дереве уже починен (`surface.rs:622-634`) | Поставить маркер без перепиновки — маркер станет разрешением не проверять |
| F-18 | RECORDED | Б§6, первый буллет | Текст исправлен на месте: файл 09 БЫЛ тронут заходом А, и это видно по маркерам на `:208`/`:246`; заход В тронул его снова | — |
| F-19 | FIXED | `.dpos-study/PLAN.md:107` | Колонка «Работа»: «`for_keys`/`for_seeds` удалены» → «НЕ удалены — перенесены в 5.1/5.2», статус и заход В — в колонке «Оценка». Хунк Э4 не тронут | Поправить только «Оценку» — строка так и будет обещать в «Работе» то, чего в дереве нет |
| F-20 | RECORDED | `.dpos-study/PLAN.md`, секция Э4 | Не 5.0 и не мой: чужая незакоммиченная работа в том же файле. Не трогал; проверено `git diff`, что мои правки её не задели | Тронуть — потеря чужой работы без следа |
| F-21 | RECORDED (a)-(e) + REJECTED (f) | (a) `beacon/mod.rs:112`; (b) `:113`; (c) `follower.rs:92`; (d) `mod.rs:107-110`; (e) `mod.rs:133-157`; (f) `surface.rs:77`, `:175`, `:218` | (a)-(e) — пять новых строк Д-9…Д-13 с причиной каждая (В§2). (f) — опровергнуто: все три типа стоят в сигнатурах экспортированного трейта | Написать Д-строки и не привязать их к ряду, который их снимает, — список отклонений снова станет неполным, как §0.4 захода А |
| F-22 | RECORDED | `beacon/surface.rs:390-396` | `unreachable!` оставлен на ОБОИХ путях (как того и требует альтернатива карты) и снабжён доказательством недостижимости по построению: `VerifiedSeed::check` отображает `Valid` в `Ok`, `verified_seed.rs:48-51`. На финализационной двери это ровно форма HEAD (`HEAD:cert_inlet.rs:3040`) | Оставить `unreachable!` без доказательства — при следующем изменении `check` паника уедет на продакшн-путь молча |
| F-23 | FIXED | `consensus/dpos.rs:3552-3558`, `:4004-4008` | Follower держит и регистрирует ОБА хэндла: `drain_on_shutdown: vec![("beacon", beacon_drain_handle)]`. Выбрано по контракту типа (`plane.rs:176-197`), а не по удобству | Убрать `drain` из `Tasks` для follower-а вместо этого — тип перестанет быть одним контрактом на два класса узла, и 5.1 придётся решать заново, где чей дренаж |
| F-24 | FIXED | `node/dpos.rs:2615-2673` | Фиксированный `ctx.sleep(50 ms)` заменён на НАБЛЮДАЕМЫЙ факт (писатель прочитал посланную запись) + `Poll::Pending` на хэндле через `noop_waker`; отдельным `assert!` закреплено, что без этого факта отрицательная половина была бы вакуумной | Заменить сон одним поллингом хэндла — отрицательная половина станет слабее сна: «не завершился» не значит «запускался» |
| F-25 | RECORDED | Б§0.2 | Текст исправлен на месте: правок в телах тестов было не одна, а четыре, и главная из них — смена ФИКСТУРЫ в `node/dpos.rs`, которую греп по `assert` не видит | — |

## В§2 Новые строки Д

| пункт | что иначе | file:line причины |
|---|---|---|
| Д-9. `agreement_partition` покидает `beacon/` | `pub(crate)`, потребитель — `epoch_manager.rs:17,573` | Имя партиции инстанса согласования нужно там же, где живёт его подметание, а подметание — это `prune_agreements`, перенос которого §7 отдаёт строке 5.4 (то же основание, что у Д-6). Закрыть имя, не перенеся подметание, значит оставить инстансы непрунутыми |
| Д-10. `absent_unregistered` покидает `beacon/` | `pub(crate)`, два продакшн-конструктора (`application.rs:377`, `cert_inlet.rs:500`) | §5.1 требует обратного порядка сборки: `CertInlet` и `FluentApp` получают `Beacon` в конструкторе, то есть `beacon::build` идёт РАНЬШЕ них. Порядок сборки в `node/src/dpos.rs` меняется вместе с этим, и §5.1 сама помечает выполнимость [ГИПОТЕЗА]. До того момента оба сайта — литералы структуры, которые `with_randomness` перекрывает в той же цепочке (`outer.rs:1150`, `node/cert_inlet.rs:113`, `consensus/dpos.rs:3941`) |
| Д-11. `ArtifactFetch` вместо `Arc<dyn ArtifactUpstream>` | Замыкание `Fn(u64) -> BoxFuture<…>` (`follower.rs:92`) | Единственная реализация — обёртка над `CertUpstream::get_epoch_artifact` (`consensus/dpos.rs`), и трейт из одного метода с одним реализатором дал бы ещё один тип на границе без второго вызывающего. Ряд 5.1 объединяет приобретение артефакта для не-члена и для follower-а одним кодом — там у поверхности появится второй потребитель, и тогда трейт будет чем оправдать |
| Д-12. `PinEffort` покидает `beacon/` | `pub`, потребители `cert_inlet.rs:18`, `epoch_manager.rs:19` | Прямое следствие Д-2: `ensure_key(epoch, PinEffort)` остаётся на трейте, пока приобретение ключа не стало чисто реактивным (работа 5.1), а аргумент метода двери не может быть приватным типом. Уходит внутрь вместе с `ensure_key`, не раньше |
| Д-13. Тестовая дверь из 25 имён (+6 под фичей) | `#[cfg(test)] pub(crate) mod testing` (`mod.rs:133-157`) | §5.1 предполагала два тестовых имени, потому что считала фикстуры стендовыми. Фактически фикстуры `executor`, `epoch_manager`, `cert_inlet`, `dpos`, `application`, `slasher` и стенда строятся над НАСТОЯЩИМИ рунгами, и живут они в тех же ФАЙЛАХ, что и продакшн-код. Дверь — это цена за то, что компилятор, а не греп, отличает тестовый доступ от продакшн-а; список сам себя сокращает по мере рядов 5.1-5.4 (Б§6) |
| Д-14. `CommitteeReads` с ОДНИМ курсором | Два: `read_at` и `qual_read_at` (`plane.rs:111`, `:134`) | §5.1 требует читать `dkgQual` на том же курсоре `max(fin, live)`, что и пару ростеров (Д-7), и это соблюдено — высота та же. Разошлись они в одном: `committee[E]` инвариантен по содержимому, поэтому его можно читать и при отсутствующем EL-финализированном маркере (откат на генезисный хэш стоит ровно одну лишнюю попытку), а бит кормит ЗАПИСЫВАЕМЫЙ ОДИН РАЗ мемо (`carry.rs:227-245`), и ответ, взятый на том откате, замораживает `false` на весь процесс. Одного метода на оба случая не хватает: «когда `None`» у них разное |

## В§3 Ворота — вывод команд

~~~
$ cargo test -p fluentbase-consensus --lib
test result: ok. 640 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 20.59s

$ cargo test -p fluentbase-consensus --lib --features dpos-devnet-byzantine testbed::
test result: ok. 35 passed; 0 failed; 0 ignored; 0 measured; 613 filtered out; finished in 31.40s

$ cargo test -p fluentbase-node --lib
test result: ok. 59 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 60.87s

$ cargo clippy -p fluentbase-consensus -p fluentbase-node --all-targets
      1 warning: large size difference between variants
      1 warning: `fluentbase-node` (lib) generated 1 warning
      1 warning: `fluentbase-node` (lib test) generated 1 warning (1 duplicate)

$ cargo clippy -p fluentbase-consensus --all-targets --features dpos-devnet-byzantine
(0 строк warning/error)

$ cargo fmt --check
(0 строк `Diff in`)

$ cargo doc -p fluentbase-consensus --no-deps 2>&1 | grep -c "unresolved link"
9
~~~

## В§4 Логические коммиты после захода В

Заход В не создаёт новых групп: каждая его правка ложится в группу того файла, который она
трогает. Обновлённый общий порядок — группы А§8 1-6, затем Б§5 1-3, и внутри них:

| путь | группа | заголовок | что добавил заход В |
|---|---|---|---|
| `beacon/{surface,certify,metrics}.rs` | А-1 / Б-1 | `refactor(beacon): give the beacon one trait and one wake-up publisher` | F-1 (снятие `notify` в `certify`), F-9, F-22, F-8 (имя `speculation` + перечень путей) |
| `beacon/{plane,follower,mod,keys,resolve}.rs` | А-2 | `refactor(beacon): build the beacon behind two constructors, one CommitteeReads and two task handles` | F-5, F-11 (`qual_read_at`), F-1 (`keys.rs`), F-13 (`keys.rs:547`), F-14 |
| `consensus/src/{spec_exec,cert_inlet,executor,epoch_manager,outer,engine,scheme,application,dpos}.rs` | А-3 | `refactor(consensus): move every beacon consumer onto the Beacon trait` | F-2, F-3, F-4, F-6, F-7, F-10 (только доки), F-13, F-23 |
| `node/src/{dpos,cert_inlet,consensus_rpc/state}.rs` | А-4 | `refactor(node): hand the beacon one CommitteeReads and supervise it as one subsystem` | F-11 (курсоры + тесты), F-12 (комментарии), F-13, F-24 |
| `consensus/src/testbed/{stand,byzantine_roles}.rs` | А-5 | `test(testbed): follow the beacon boundary` | F-11 (`StandCommitteeReads::qual_read_at`) |
| `.claude/dpos_architecture/*`, `.dpos-study/*` | А-6 / Б-3 | `docs(dpos): record the beacon boundary` | F-15, F-16, F-17, F-18, F-19, F-25, эта часть В, запись `verified-against` |

Оговорка А§8 остаётся в силе: группы 1 и 2 по отдельности не компилируются и должны быть
одним коммитом. Заход В её не ухудшил и не улучшил — новый `qual_read_at` объявлен в
`plane.rs` (группа 2) и реализован в `node`/`consensus/dpos`/`stand` (группы 3-5), то есть
зелёная точка по-прежнему после группы 2, и группы 3-5 обязаны идти вместе с ней в том же
порядке, что и раньше.

## В§5 Что осталось для 5.1-5.4

- **5.1.** Снять `for_keys` (потребители — только два теста в `surface.rs`) и перенести
  `PinEffort` внутрь вместе с `ensure_key` (Д-12). Поменять порядок сборки так, чтобы
  `beacon::build` шёл перед `CertInlet::new`/`FluentApp::new`, и тогда снять
  `absent_unregistered` (Д-10). Второй потребитель у артефактного приобретения превратит
  `ArtifactFetch` в трейт (Д-11). Курсор `qual_read_at` исчезнет вместе с `carry.rs`-мемо
  или останется как есть — решать там, где будет виден новый владелец бита (Д-14).
- **5.2.** Дать `Observed` читателей на всех трёх сайтах `let _observed` (`spec_exec.rs:92`,
  `cert_inlet.rs:860`, `:3129`) — синхронный `Refused` в data fault, `Pending` в
  `ReplaySeed::Defer`. Снять `waiters`/`wait_for`/`prune_waiters` из `certify.rs` (сейчас
  `#[cfg(test)]`, но `record` всё ещё обходит карту на каждом вызове). Снять `for_seeds`
  вместе с фикстурой executor-а.
- **5.3.** Появится производитель `Stalled` — и тогда `BeaconEvent` перестанет быть
  «три варианта без нагрузки» (Д-4/Д-5).
- **5.4.** `agreement_intake` и `agreement_partition` уходят внутрь вместе с подметанием
  (Д-6, Д-9). Тогда же `Tasks` станет ровно двумя полями, как требует §5.1.
- **Вне рядов, одной строкой каждое.** `cert_follow.rs:107` всё ещё называет
  `beacon::for_follower` — файл вне списка этой задачи. `cert_follow.rs:95` ссылается на
  `beacon::decode_artifact`, которого в двери нет ни на HEAD, ни в дереве.
  `plane_upstream.rs:186` ссылается на `beacon::log_resolver::LogHandler` — после
  приватизации подмодулей это ссылка в закрытую комнату. `application.rs` держит поле
  `randomness`, которое никто не читает (предсуществует).

## В§6 Где проверка была слабее всего

1. **F-12 остаётся [ГИПОТЕЗА] по ВРЕМЕНИ.** Я перечислил всех четырёх держателей
   `Arc<dyn Beacon>` и показал по коду, что ни один не переживает `supervise` (кроме
   хостового, который дропается явно, и RPC-замыкания, которое `Weak`). Чего я НЕ показал:
   что `Handle::abort` успевает ДРОПНУТЬ future аборченной задачи до того, как узел войдёт
   в `drain_shutdown_tasks` — tokio дропает future аборченной задачи на своём планировщике,
   асинхронно. Ни один тест этого не достаёт, живого узла я не запускал. Это ровно та
   [ГИПОТЕЗА], которую признавало и ревью (§9.1), и заход В её не закрыл.
2. **F-11: эксплуатируемость не построена.** Я закрыл окно и покрыл его тестом на
   арифметику курсоров, но девнета, где `committee[E]` для `E ≥ DETERMINISTIC_BOOTSTRAP_EPOCH`
   закоммичен в генезисе и узел стартует с `live > 0` при отсутствующем finalized-маркере,
   я не строил. Что мемо там засеялось бы `false` — вывод по коду `carry.rs:227-245`, не
   наблюдение.
3. **F-5 проверена рассуждением о дроп-глу, а не тестом.** Что `SupervisedChildren`
   теперь захвачена в future, гарантирует язык; что `abort()` до первого poll-а действительно
   дропает future (а не оставляет её висеть), я взял из `runtime/src/tokio/runtime.rs:575-578`
   для ветки `aborted` и из семантики `tokio::task::JoinHandle::abort` — теста на
   «аборт до первого poll-а убил шестерых детей» я не написал.
4. **F-3: такт снят не полностью, и это названо в В§0.5.** Задача epoch_manager-а
   по-прежнему ПОЛЛИТСЯ раз в раунд; не завершается только плечо. Замерить разницу
   (профилем, счётчиком пробуждений) я не пробовал — аргумент чисто структурный.
5. **F-10 доказан по reth, но не прогоном.** Цепочка `finalized_block_number` →
   `get_finalized_num_hash` прочитана в пинованном чекауте; что на живом узле две
   поверхности ни разу не разошлись, я не наблюдал.
6. **Стенд я не сверял построчно.** Единственная правка стенда захода В —
   `StandCommitteeReads::qual_read_at` (делегирует `read_at` с доком о том, почему стенд
   не может выразить производственную гвардию). Опираюсь на прогон 35/0, не на построчную
   сверку.

**Счётчики текстовых проверок.** По ВСЕМУ файлу журнала (не только по части В, потому что
заход В правил текст и в А, и в Б): строк, оканчивающихся на многоточие — 0; строк с
нечётным числом обратных кавычек — 0; удвоенных запятых — 0; пустых пар обратных кавычек —
0; пустых круглых скобок вне код-спанов — 0. По `PLAN.md:107` — те же пять нулей. По
правленым `.md` в `.claude/dpos_architecture/` считал ТОЛЬКО свои абзацы, и там те же пять
нулей (одну разорванную переносом пару `` cargo fmt --check `` в собственной записи
`verified-against` нашёл этой проверкой и переформулировал); по этим файлам ЦЕЛИКОМ счёт не
нулевой — в 00, 09 и 15 остаются пред-существующие код-спаны, перенесённые через конец
строки, и они не мои. Считано скриптом на python.
