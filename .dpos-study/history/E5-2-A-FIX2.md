# Э5, строка 5.2, заход А — малый проход (доккомментарии в КОДЕ, фаза Ф7)

База `HEAD` = `37691136`, ветка `djadjka/dpos-reth-2.2-squashed`. Правки только в
комментариях; ни строки исполняемого кода, ни одного теста. Git — только на чтение.

## §0 — таблица

| пункт | сделано / не сделано | file:lines (после правки) | чем проверено ТЕЛО |
|---|---|---|---|
| 1. `beacon/mod.rs:40-41`, «PLAN rows 5.1-5.2 give the last two their readers» в будущем времени | сделано | `beacon/mod.rs:36-49` | читатели `Observed`: `spec_exec.rs:123` (`if matches!(observed, Observed::Refused \| Observed::Pending)`), `cert_inlet.rs:733-736` (live-поток, `== Observed::Refused` → `record_data_fault`), `cert_inlet.rs:3030-3032` (`impl UpstreamResolver`, объявлен на `cert_inlet.rs:2905`, т.е. ПОСЛЕ закрытия `#[cfg(test)] mod tests` с `:885`), `dpos.rs:741-759` (`seed_via_beacon`, все четыре варианта). Читатели `DataFault`: поле `cert_inlet.rs:406`, подписка `:472`, слив `drain_late_verdicts` `:562` и `:847` |
| 2. `beacon/mod.rs:45-46`, «the two PRODUCTION implementations» | сделано | `beacon/mod.rs:53-59` | `git grep -n 'Randomness for' -- crates` → ровно одна строка: `beacon/surface.rs:2194 impl Randomness for LiveBeacon`; сам трейт `surface.rs:558` |
| 3. `beacon/seed_journal.rs:7-10`, мёртвое обоснование через certify-гейт | сделано | `beacon/seed_journal.rs:7-25` | `spec_exec.rs:56-85` — перевыведенный контракт целиком прочитан: три шага «что защищает СЕЙЧАС» (`:61-74`) и «WHAT IT DOES NOT PROTECT» про «precede the executor send» (`:77-85`). `certify.rs` удалён (`git status`: ` D crates/dpos/consensus/src/beacon/certify.rs`). Имя `record` живо: `beacon/seed_index.rs:194` |
| 4. `beacon/log_resolver.rs:128-131`, «`SeedStore::insert` writes the per-epoch terminal pin» + несуществующий `capture_certificate_seed` | сделано, но **вывод переписан НЕ так, как в постановке** — см. §1 | `beacon/log_resolver.rs:127-149` | `git grep -n 'capture_certificate_seed' -- crates` → единственный хит — сам этот комментарий. Прежний пин: `git show HEAD:…/beacon/certify.rs` `:201-203` (`insert` → `pin_terminal`), `:376-387` (тело), `:409-413` (`retain_terminal_from`). Новое правило: `beacon/seed_index.rs:512-535` (`oldest_evictable`), `:402` (`floor = top_epoch − SCHEME_RETENTION_EPOCHS`), `:472-486` (`bound_verified`). Одна дверь записи: `beacon/seed_index.rs:1-8` |
| 5. `executor.rs:9475-9476`, ссылка на `certify.rs`'s `seed_store_record_notifies_without_a_lost_wakeup` | сделано | `executor.rs:9479-9487` | преемник: `beacon/seed_index.rs:634 fn seed_index_record_notifies_without_a_lost_wakeup`; старого имени в дереве нет (`grep -rn 'notifies_without_a_lost_wakeup'` даёт только `seed_index.rs:634` и `:684`) |
| 6. «`Notify`» вместо broadcast — три места | сделано в трёх, но **анкеры `:1689-1690` и `:5219` не те** — см. §1 | `executor.rs:1691-1699`, `:2485-2492`, `:9479-9481` | механизм: `beacon/surface.rs:378-380` (`enum BeaconEvent { SeedRecorded }`), `:245 fn subscribe(&self) -> broadcast::Receiver<BeaconEvent>`, отправка `beacon/seed_index.rs:256 self.events.send(BeaconEvent::SeedRecorded)`. Ловушка: `beacon/surface.rs:236-241` («SUBSCRIBE BEFORE THE FIRST READ… drops a send with no receiver, where the `notify_one` permits this replaces were stored»). Приёмник у экзекьютора: `executor.rs:1350 let mut beacon_events = self.randomness.subscribe();` |
| 7. `dpos.rs:773`, `[`seed_from_cert`]` | сделано | `dpos.rs:773` | функция: `dpos.rs:741 fn seed_via_beacon`; вызовы `:819`, `:836`. `git grep 'seed_from_cert' -- crates` после правки — только `dpos.rs:4689` (там «предшественник», не тронуто) |
| 8. `beacon/plane.rs:1041`, буллет «no geometry watch» | сделано | `beacon/plane.rs:1036-1045` | конструктор: `beacon/plane.rs:1206-1210` — `// A follower freezes no (activation, interval)… GeometryUnfrozen refinement — must not fire` и `geometry: watch::channel(Some((0, 1))).1` |
| 9. `beacon/mod.rs:177`, «alias **below** it» | сделано | `beacon/mod.rs:189` | алиас `pub(crate) use super::seed_index::SeedIndex as SeedStore;` — `beacon/mod.rs:182` (ВЫШЕ правленой строки `:189`) |

Изменённые файлы: `beacon/mod.rs`, `beacon/seed_journal.rs`, `beacon/log_resolver.rs`,
`beacon/plane.rs`, `executor.rs`, `dpos.rs`. Больше ничего.

## §1 — где я отошёл от постановки (правок КОДА не потребовалось нигде)

Ни один из девяти пунктов не потребовал правки исполняемого кода. Но по двум
пунктам постановка неверна по телу, и я это не воспроизвёл.

**П. 4 — «чем пин ограничен не был».** Пин БЫЛ ограничен ровно тем же окном.
`git show HEAD:crates/dpos/consensus/src/beacon/surface.rs` `:2416-2427`:

```
fn observe_epoch(&self, _reconciled: Epoch, entered_frontier: Epoch) {
    let oldest = entered_frontier.get().saturating_sub(crate::SCHEME_RETENTION_EPOCHS as u64);
    …
    // And so does the terminal pin: it is asked for by the NEXT epoch, so an
    // epoch past the retention edge has no asker left.
    self.seeds.retain_terminal_from(oldest);
}
```

и то же в `observe_cert` (`:2429-2433`); тело `retain_terminal_from` —
`git show HEAD:…/beacon/certify.rs:409-413`. То есть «окно `SCHEME_RETENTION_EPOCHS`»
— НЕ новая часть, и написать «пин им ограничен не был» значило бы поставить в код
утверждение, опровергаемое одним `git show`.

Ослабление, которое я в итоге записал (и которое проверяется по телам): раньше
вывод опирался на ЗАПИСЬ — тот же вызов клал ВТОРУЮ копию раунда в отдельную карту
`terminal` (`certify.rs:201-203` → `:376-387`), у читателя границы было собственное
хранилище, и счётчик σ-по-раундам до него не доставал. Второй копии больше нет:
σ живёт в единственной карте `round → σ` и выживает только потому, что ПРАВИЛО
вытеснения её пропускает (`seed_index.rs:512-535`, вызывается из `bound_verified`
`:472-486`). Отрицательное свойство («никто её не удаляет») вместо положительного
(«кто-то записал её в другое место»), под одним общим бюджетом вместо двух. Это и
записано, вместе с явным «окно — не новая часть», чтобы следующий читатель не
переоткрыл ту же ошибку.

**П. 6 — анкеры.** `:1689-1690` и `:5219` в рабочем дереве — это `}` и пустая
строка (`executor.rs:1689` = `}`, `:1690` = пусто; `:5219` — середина выражения
`deal_anonymous::<MinSig, N3f1>(…)` в тестовом хелпере, не комментарий). Похоже, это
номера ХАНКОВ из `git diff HEAD -- crates/dpos/consensus/src/executor.rs`
(`@@ -1689,7 +1689,7 @@`, `@@ -2479,7 +2479,7 @@`), а не строк комментария.
Третьего ханка на `5219` нет вовсе — список ханков:
`752, 1689, 2479, 2984, 4252, 4263, 5009, 5048, 7695, 8230, 8285, 9068, 9445`.

Что я сделал вместо этого: правил три места, где механизм НАЗВАН неверно про
ТЕКУЩИЙ код и которые при этом лежат в диффе 5.2 (там же менялось `SeedStore` →
`SeedIndex`/«seed index» в том же предложении):

* `executor.rs:1691` (ханк `1689`) — «Seed-record notify arm» → назван
  `broadcast::Sender<BeaconEvent>` + `SeedRecorded`, с ловушкой из `surface.rs:236-241`;
* `executor.rs:2488` (ханк `2479`) — «`SeedIndex`'s per-record `Notify`»;
* `executor.rs:9479-9481` (ханк `9445`) — «(which fires the `Notify`)». Это же место
  правится по п. 5, и в диффе 5.2 в той же фразе менялось `SeedStore` → `shared
  seed index` — по описанию п. 6 подходит точно.

**П. 7 — ворота `cargo doc` не подвинулись.** `unresolved link` осталось **6**, не
меньше. Причина: `seed_via_beacon`/`seed_from_cert` — приватная функция в
приватном модуле, `cargo doc --no-deps` без `--document-private-items` её доккоммент
не обрабатывает, поэтому сломанная ссылка никогда и не считалась. В логе прогона
`seed_from_cert` не встречается ни разу. Шесть оставшихся — все чужие и
предсуществующие: `beacon/mod.rs:2` (`E`), `cold_start_jump.rs:775` (`E`),
`engine.rs:144` (`crate::epoch_manager::Actor::enter`), `executor.rs:368` и `:389`
(`FakeMarshal`), `slasher/evidence.rs:510` (`Scheme::verify_attestation`).

## §2 — ворота verbatim

```
$ cargo fmt --check
(только Warning'и «unstable features are only available in nightly channel»)
exit 0

$ CARGO_BUILD_JOBS=6 cargo test -p fluentbase-consensus --lib
test result: ok. 665 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 172.55s

$ CARGO_BUILD_JOBS=6 cargo clippy -p fluentbase-consensus --all-targets
(после `touch crates/dpos/consensus/src/lib.rs` — пересборка крейта)
Finished `dev` profile [unoptimized + debuginfo] target(s)
ноль warning/error в fluentbase-consensus

$ CARGO_BUILD_JOBS=6 cargo doc -p fluentbase-consensus --no-deps
unresolved link: 6   (было 6; см. §1, п. 7 — почему не стало меньше)
всего warning'ов в логе: 59
```

Стенд, `-p fluentbase-node`, reader и `--test slasher_integration` не гонял —
правки чисто комментарные, и в машине идёт docker-прогон девнета.

## §3 — что ещё того же класса нашёл по дороге (НЕ правил)

1. **`executor.rs:7567-7568` — четвёртое «`Notify`» про текущий механизм.**
   «Recording σ into the store the actor holds fires the seed-record Notify, and
   the executor's REAL `seed_notify` select! arm…». Тот же дрейф, что п. 6, но
   этой строки нет в диффе 5.2 (ханков между `5048` и `7695` нет), поэтому под
   «во всех трёх» я её не подводил. Правка — одна фраза. Имя самого теста
   (`executor.rs:7572 a_held_block_derives_when_its_seed_lands_through_the_real_notify_arm`)
   — код, не трогал.
2. **Исторические «`Notify`» — КОРРЕКТНЫ, не трогать.** `executor.rs:204`
   («the park the `Notify` shape had for free»), `:1354` и `:13067` («The HEAD
   shape parked on a `Notify` whose sender it also held») говорят про ПРОШЛУЮ
   форму явно и в прошедшем времени. Если кто-то будет чистить класс греп-ом
   по `Notify`, эти три — ложные срабатывания.
3. **`beacon/surface.rs:220-231` — обещание в будущем времени, которое строка уже
   наполовину выполнила.** «NO PRODUCTION CALLER is left (review C-10)… It is
   kept on the trait only by the delegating wrapper in
   `testbed/byzantine_roles.rs:404`, which row 5.2 may not write; 5.4 deletes
   both» — про `observe_cert`. Утверждение о количестве («no production caller»)
   плюс обещание («5.4 deletes both»): на момент 5.4 обе половины надо
   перепроверить, иначе получится ровно тот же мёртвый текст.
4. **`beacon/mod.rs:176-182` (сам алиас `SeedStore`) — утверждение о количестве с
   точными анкерами в чужом файле.** «`epoch_manager.rs`'s test module names it
   (`epoch_manager.rs:2076`, `:2080`, `:2129`, `:2836`, `:2848`)» — пять номеров
   строк в файле вне write-list 5.2. Любая правка `epoch_manager.rs` их сдвинет,
   и это единственное обоснование существования алиаса. Класс тот же: текст,
   опровергаемый однострочным грепом.
5. **`beacon/plane.rs:1033-1035` — «a follower has no epoch manager, so the
   `Pending` verdict is the ONLY thing that can ask for a key».** Утверждение о
   количестве («ONLY»); я его НЕ проверял по телам — оставляю как кандидата для
   следующего прохода, не как находку.
6. **Процедурное.** Я один раз задел `**/.claude/session-reads/*.jsonl`
   рекурсивным `grep -rn 'seed_notify' crates/dpos/consensus/src` — вывод попал в
   контекст. Дальше искал только по `git grep` / точечным путям. На правки это
   не повлияло: ни одна правка не опирается на содержимое этих файлов.
