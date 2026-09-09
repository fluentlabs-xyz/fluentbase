# BLS12381Verifier: спецификация verify и подготовка к инлайну

Разведка перед задачей, где верификатор вливается в стейкинг-контракт, а внешний
подменяемый адрес удаляется. Сам инлайн здесь не делается.

Дата прохода: 2026-09-08. Деревья:
- **A** — `/home/djadjka/Work/audit-482/pr482-study`, ветка `feat/flu-989-port-solidity-delta`,
  HEAD `100c02c4`.
- **B** — `/home/djadjka/Work/fluentbase`, ветка `djadjka/dpos-reth-2.2-squashed`.
- **C** — `/home/djadjka/Work/solidity-contracts` (соседний репозиторий солидити;
  найден в этом проходе, в исходной постановке его не было).

Скретч-каталог прохода:
`/tmp/claude-1000/-home-djadjka-Work-fluentbase/23eac771-f2c3-4cc6-aeb4-88fbc4763742/scratchpad`.

---

## 1. Вопрос об исходнике и решение

### 1.1 Исходник существует и найден

**Утверждение.** `BLS12381Verifier.sol` не потерян: он лежит в третьем дереве
C (`solidity-contracts`) ровно на том коммите, который записан в `.vendor-sha`.

**Опора.**
- `devnet/local-dpos-smoke/contracts/.vendor-sha` = `f641789fcd69751213c3f2f03725321488a4a744`
  (прочитан файл).
- `git -C /home/djadjka/Work/solidity-contracts cat-file -t f641789f…` → `commit`.
- `git show f641789f:contracts/libraries/BLS12381Verifier.sol` → 12869 байт,
  сохранён как `scratchpad/orig.sol`.
- `cast keccak 0x<содержимое orig.sol>` →
  `0x3043635241d50a84a36e11b989ed59370a5caa4594727ccfa1086fd01aae479e`,
  что побайтно совпадает с `metadata.sources["contracts/libraries/BLS12381Verifier.sol"].keccak256`
  из артефакта `BLS12381Verifier.json`.
- Компиляция этого файла `solc 0.8.30` с настройками из `rawMetadata`
  (`optimizer.enabled=true, runs=80, viaIR=true, evmVersion=prague`) даёт
  `deployedBytecode` длиной 3876 байт, из которых первые **3823 совпадают
  побайтно** с артефактом; расходится только 53-байтный CBOR-хвост
  (IPFS-хеш метаданных зависит от remappings, которых у меня в standard-json нет).

**Уверенность.** `[KNOWN]`

**Чем пытался опровергнуть.** Сравнил не только keccak исходника, но и
скомпилированный runtime-код; проверил, что длина файла (12869) совпадает с
диапазоном `SourceUnit.src = "39:12830:32"` из AST артефакта (39+12830 = 12869).
Три независимых совпадения (keccak, длина по AST, 3823 байта кода) — совпасть
случайно не могут.

**Если порт сделают наполовину правильно здесь.** Будут реверсить байткод там,
где есть исходник, и почти наверняка потеряют что-то в краевых ветках
(`_rejectInfinity` по всей длине, точный порядок `xc1‖xc0`, строгое `>` в
сравнении с `(p-1)/2`).

### 1.2 Где именно он лежит и где его нет

**Утверждение.** Файл несут ровно четыре ссылки в C, и ни одна из них не `devel`;
на `HEAD` текущего чекаута C файла нет.

**Опора.** Перебор всех 37 локальных и удалённых ссылок C через
`git rev-parse "<ref>:contracts/libraries/BLS12381Verifier.sol"`:

| blob | ссылки |
|---|---|
| `a14c5f775ad3ff2988798b9260f6f84225b5aecb` (12869 B, актуальный) | `refs/heads/djadjka/bls-staking`, `refs/heads/djadjka/dpos-audit-fixes`, `refs/remotes/origin/djadjka/bls-staking`, `refs/remotes/origin/djadjka/dpos-audit-fixes` |
| `9ba1800d0e075bba5006dd2ec5bc1660fc9e9e34` (10973 B, старый) | `refs/heads/fix/staking-contracts` |

`git ls-tree HEAD contracts/libraries/` на текущей ветке C (`djadjka/rollup-audit`)
показывает только `ExcessivelySafeCall.sol`, `Heap.sol`, `MerkleTree.sol`.

**Уверенность.** `[KNOWN]`

**Чем пытался опровергнуть.** Первая попытка того же перебора через shell-цикл
`while read` вернула пустой вывод — я его НЕ принял за отрицательный результат и
переписал перебор на Python; вторая версия нашла четыре ссылки. Пустой вывод был
артефактом оболочки, а не фактом.

**Если порт сделают наполовину правильно здесь.** Возьмут блоб
`9ba1800d` (10973 байта) с ветки `fix/staking-contracts` как «исходник» — это
более ранняя редакция, и она НЕ соответствует развёрнутому артефакту.

### 1.3 Пятая копия исходника

**Утверждение.** В C есть ещё одна копия того же файла, побайтно совпадающая с
вендоренной версией: `C/.claude/tasks/2026_08_31__20_37__drand_randomness_oracle/BLS12381Verifier.snapshot.sol`.

**Опора.** `diff -q <snapshot> scratchpad/orig.sol` → файлы идентичны.

**Уверенность.** `[KNOWN]`

### 1.4 В дереве B исходника нет — и это уже неважно

**Утверждение.** В полной истории B `.sol` действительно отсутствует; есть только
артефакт и rWasm-реализация предеплоев.

**Опора.** `git -C B rev-list --all --objects | grep -i BLS12381` (обход ВСЕХ
достижимых объектов по всем 4150 коммитам, не первые 3000) даёт ровно:
`devnet/local-dpos-smoke/contracts/BLS12381Verifier.json`,
`contracts/bls12381/**` (rWasm-предеплой), `crates/crypto/src/bls12381.rs`.
Ни одного `.sol` с этим именем.

**Уверенность.** `[KNOWN]`

**Непроверенное.** Недостижимые (dangling) объекты в B и ссылки, которых нет
локально, не проверялись.

### 1.5 Артефакт несёт полный AST — исходник восстановим и без репозитория C

**Утверждение.** `BLS12381Verifier.json` содержит полное дерево разбора (914 узлов,
13 `FunctionDefinition`, **ноль** `InlineAssembly`), поэтому исходник
восстанавливается из артефакта механически, без реверса байткода.

**Опора.** Обход `ast` артефакта с подсчётом `nodeType`; печатник AST→Solidity
(`scratchpad/unast.py`, 236 строк вывода, ноль непечатанных узлов) даёт файл,
который компилируется теми же настройками и даёт **те же 3823 байта кода**, что и
артефакт (`scratchpad/BLS12381Verifier.recon.sol`).

**Уверенность.** `[KNOWN]`

**Чем пытался опровергнуть.** Специально считал `InlineAssembly` — если бы в
контракте был Yul, AST-печатник молча выронил бы его тело и совпадение байткода
не состоялось бы. Совпадение состоялось.

### 1.6 Решение

Порт делается **из исходника** `C@f641789f:contracts/libraries/BLS12381Verifier.sol`
(идентичного `scratchpad/orig.sol`), а не реверсом. Реверс байткода в этой задаче
не нужен ни для одного пункта. Артефакт остаётся вторым независимым источником
(через AST), производящая сторона узла — третьим.

Отдельно: в C на той же ветке лежат готовые конформанс-тесты солидити, которые
порт обязан унаследовать —
`test/bls/BlsHashToG1Conformance.t.sol`, `test/bls/Eip2537Conformance.t.sol`,
`test/bls/Eip2537ConformanceVectors.sol`, `test/bls/BLS12381VerifierHarness.sol`,
`contracts/staking/interfaces/IBLS12381Verifier.sol` (`git ls-tree -r --name-only
djadjka/dpos-audit-fixes | grep -i bls`).

---

## 2. Спецификация verify (побайтно)

Всё ниже — из `scratchpad/orig.sol` (он же `C@f641789f:contracts/libraries/BLS12381Verifier.sol`),
номера строк по этому файлу. Все утверждения этого раздела `[KNOWN]`: файл прочитан,
и каждое поведение дополнительно проверено на развёрнутом контракте (см. §4).

### 2.1 Пять аргументов и порядок

```
verify(bytes namespace, bytes message, bytes dst, bytes sigUncompressed, bytes pkUncompressed)
    external view returns (bool)                              // orig.sol:52-58
```
Селектор `0x8bf26133`. Схема MinSig: подпись в G1 (128 B EIP-2537), ключ в G2
(256 B EIP-2537).

### 2.2 Сборка сообщения под подпись

```
h = _hashToG1( unionUnique(namespace, message), dst )         // orig.sol:65
unionUnique(ns,msg) = bytes1(uint8(ns.length)) ‖ ns ‖ msg     // orig.sol:80-83
```
Порядок склейки — ровно `len ‖ ns ‖ msg`, один байт длины. Если `ns.length >= 0x80`
— `revert NamespaceTooLong()` (`0xcfa39070`). Длина `msg` не ограничена.

### 2.3 DST

DST приходит **аргументом**, контракт его не знает. Внутри:
```
if (dst.length > 255) revert DstTooLong();                    // orig.sol:90
dstPrime = dst ‖ bytes1(uint8(dst.length))                    // orig.sol:93
```
Длинный DST (RFC 9380 §5.3.3, `H("H2C-OVERSIZE-DST-"‖DST)`) НЕ реализован —
контракт просто ревертит. Пустой DST (`length == 0`) проходит и даёт `DST' = 0x00`.

### 2.4 Хеш в G1: expand_message_xmd(SHA-256), затем два map+add

```
z     = 64 нулевых байта                                      // orig.sol:97
msgP  = z ‖ input ‖ 0x0080 ‖ 0x00 ‖ dstPrime                  // orig.sol:98
b0 = sha256(msgP)                                             // orig.sol:100
b1 = sha256(b0        ‖ 0x01 ‖ dstPrime)                      // orig.sol:101
b2 = sha256(b0^b1     ‖ 0x02 ‖ dstPrime)                      // orig.sol:102
b3 = sha256(b0^b2     ‖ 0x03 ‖ dstPrime)                      // orig.sol:103
b4 = sha256(b0^b3     ‖ 0x04 ‖ dstPrime)                      // orig.sol:104
uniform = b1‖b2‖b3‖b4                       (128 B)           // orig.sol:107
u0 = MODEXP(uniform[0:64],  1, p)           (48 B)            // orig.sol:110
u1 = MODEXP(uniform[64:128],1, p)           (48 B)            // orig.sol:111
P0 = MAP_FP_TO_G1(16×0x00 ‖ u0)             (128 B)           // orig.sol:114
P1 = MAP_FP_TO_G1(16×0x00 ‖ u1)             (128 B)           // orig.sol:115
H  = G1ADD(P0 ‖ P1)                         (128 B)           // orig.sol:118
```
`0x0080` — это `I2OSP(len_in_bytes=128, 2)`; `ell = 128/32 = 4`; `s_in_bytes = 64`
(блок SHA-256). Это в точности RFC 9380 `expand_message_xmd` и `hash_to_field(m=1,
count=2, L=64)`.

Отличие от буквы RFC 9380: там `clear_cofactor` применяется ОДИН раз после
сложения (`P = clear_cofactor(Q0+Q1)`), а здесь очистка кофактора сидит внутри
каждого `MAP_FP_TO_G1` — то есть считается `h·Q0 + h·Q1`. Это то же самое, потому
что умножение на скаляр `h_eff` линейно. `[LIKELY]` как рассуждение — но
`[KNOWN]` как факт: результат совпал с blst на всех шести векторах (§4.3).

### 2.5 Вызовы предеплоев и calldata

| порядок | адрес | calldata | ожидаемый ответ |
|---|---|---|---|
| 1 | `0x02` SHA256 | `msgP`, потом 4 × (32+1+len(dstPrime)) | 32 B |
| 2 | `0x05` MODEXP ×2 | `I2OSP(64,32) ‖ I2OSP(1,32) ‖ I2OSP(48,32) ‖ base(64) ‖ 0x01 ‖ p(48)` | ровно 48 B, иначе `PrecompileFailed` |
| 3 | `0x10` MAP_FP_TO_G1 ×2 | `16×0x00 ‖ fp(48)` = 64 B | ровно 128 B, иначе `PrecompileFailed` |
| 4 | `0x0b` G1ADD | `P0(128) ‖ P1(128)` = 256 B | ровно 128 B, иначе `PrecompileFailed` |
| 5 | `0x0f` PAIRING | `sig(128) ‖ NEG_G2_GENERATOR(256) ‖ H(128) ‖ pk(256)` = **768 B** | 32 B, значение `1` |

`sha256` вызывается встроенным оператором Solidity, который НЕ проверяет успех —
для адреса `0x02` это безопасно.

### 2.6 Проверяемое уравнение

```
input = sigUncompressed ‖ NEG_G2_GENERATOR ‖ h ‖ pkUncompressed      // orig.sol:70
(ok, out) = PAIRING.staticcall(input)                                // orig.sol:71
return ok && out.length == 32 && bytes32(out) == bytes32(uint256(1)) // orig.sol:72
```
То есть `e(sig, −G2gen) · e(H, pk) == 1`.

**NEG_G2_GENERATOR действительно равен −G2gen.** `[KNOWN]`
Опора: покоординатно `x.c0`, `x.c1` совпадают с генератором G2, а
`y.c0`, `y.c1` равны `p − y` генератора (посчитано в Python); и независимо —
`PAIRING(G1gen‖G2gen ‖ G1gen‖NEG_G2_GENERATOR)` на развёрнутом предеплое вернул
`0x…01`, то есть `e(G,G2)·e(G,NEG) = 1`.

### 2.7 Формат точек

- **G1 (подпись, H)**: 128 B = `pad16 ‖ x(48) ‖ pad16 ‖ y(48)`, каждый Fp
  big-endian. Ровно EIP-2537.
- **G2 (ключ)**: 256 B = `pad16‖x.c0 ‖ pad16‖x.c1 ‖ pad16‖y.c0 ‖ pad16‖y.c1` —
  **вещественная часть первой** (порядок EIP-2537), в отличие от z-cash/blst,
  где мнимая первая.
- **Сжатый вид на выходе `compress*`**: z-cash. G1 — 48 B, G2 — 96 B и там
  `x.c1 ‖ x.c0`, **мнимая часть первой** (`orig.sol:214`).

Порядок координат меняется между входом и выходом — это самое лёгкое место для
ошибки в порте.

### 2.8 Когда `false`, а когда `revert`

Проверено вызовами на развёрнутом контракте (§4.2), не только чтением.

`revert` (то есть транзакция откатывается):

| условие | ошибка | селектор | строка |
|---|---|---|---|
| `sig.length != 128` или `pk.length != 256` | `InvalidPointLength()` | `0x3532eb3b` | 61 |
| все 128/256 байт точки нулевые | `InfinityPoint()` | `0x5a3dde75` | 62-63, 170-175 |
| `H` вышла нулевой | `InfinityPoint()` | `0x5a3dde75` | 66 |
| `namespace.length >= 128` | `NamespaceTooLong()` | `0xcfa39070` | 81 |
| `dst.length > 255` | `DstTooLong()` | `0x8c978650` | 90 |
| MODEXP/MAP/G1ADD дали не тот размер или упали | `PrecompileFailed()` | `0x84e81692` | 111,134,143 |

`false` (тихий отказ):
- PAIRING вернул не `1` — неверная подпись, чужой ключ, чужой namespace, чужой DST;
- точка не на кривой / не в подгруппе — PAIRING отвергает;
- 16-байтный паддинг EIP-2537 не нулевой — PAIRING отвергает;
- координата ≥ p — PAIRING отвергает;
- PAIRING вернул `ok=false` или длину не 32.

**Практический вывод для порта:** ВСЁ, что относится к «точка плохая», приходит
как `false`, а не как revert. Единственный «плохой вход», который ревертит, —
бесконечность (все байты нули) и неверная длина.

### 2.9 compressG1Unchecked / compressG2Unchecked и цена слова «Unchecked»

```
compressG1Unchecked(bytes128) -> bytes48                      // orig.sol:184-194
  x = in[16:64]; y = in[80:128]
  если x==0 && y==0 -> revert InfinityPoint
  x[0] |= 0x80 | (y > (p-1)/2 ? 0x20 : 0)
  return x

compressG2Unchecked(bytes256) -> bytes96                      // orig.sol:199-217
  xc0=in[16:64]; xc1=in[80:128]; yc0=in[144:192]; yc1=in[208:256]
  если все четыре == 0 -> revert InfinityPoint
  sign = (yc1 > (p-1)/2) || (yc1 == 0 && yc0 > (p-1)/2)
  cand = xc1 ‖ xc0
  cand[0] |= 0x80 | (sign ? 0x20 : 0)
  return cand
```
Сравнение с `(p-1)/2` — 384-битное беззнаковое, разбитое на старшие 16 байт и
младшие 32 (`HALF_HI`/`HALF_LO`, `orig.sol:29-30`). Я пересчитал `(p-1)/2` в
Python: `0d0088f51cbff34d258dd3db21a5d66b ‖ b23ba5c279c2895fb39869507b587b12
0f55ffff58a9ffffdcff7fffffffd555` — константы совпадают. Сравнение **строгое**:
на `y == (p-1)/2` знаковый бит НЕ ставится (проверено вызовом: первый байт `85`),
на `(p-1)/2 + 1` — ставится (`a5`). `[KNOWN]`

**Что НЕ проверяется** (проверено вызовами, §4.4):
1. **on-curve** — `x=1, y=1` спокойно сжимается в `0x80…01`.
2. **принадлежность подгруппе** — вообще не смотрится.
3. **16-байтный паддинг EIP-2537** — байты `[0:16]`, `[64:80]` (и для G2
   `[128:144]`, `[192:208]`) **полностью игнорируются**. Проверено: вход с
   паддингом `0xff` даёт тот же 48-байтный результат, что и корректный вход.
4. **каноничность Fp (`< p`)** — не проверяется, и это даёт **настоящую
   коллизию**: вход с `x[0]=0x25` (то есть `x > p`) и малым `y` и вход с
   `x[0]=0x05` (`x < p`) и `y = p-1` дают **один и тот же** 48-байтный результат
   `0xa500…07`. Проверено вызовом на развёрнутом контракте.

**Что из этого следует для связи «сжатый ключ ↔ владелец».**
В контракте личность берётся так (A:`consensus.rs:1016-1021`):
```
supplied_key = compressG2Unchecked(command.pk_uncompressed)
validator    = bls_pubkey_owner[keccak256(supplied_key)]
```
То есть по НЕПРОВЕРЕННОМУ сжатию. Атакующий может подать 256 байт, которые
сожмутся в чужой зарегистрированный ключ (через паддинг-мусор или через
неканоничный `x`), и получить `validator = жертва`. Но следом идёт
`verify(..., pk_uncompressed)` с ТЕМИ ЖЕ 256 байтами, а PAIRING отвергает и
ненулевой паддинг, и `x ≥ p` — я это проверил, оба случая дают `false`
(§4.2). Значит путь закрывается **вторым** шагом, а не сжатием.

**Уверенность.** `[KNOWN]` (и коллизия, и то, что PAIRING её ловит).

**Чем пытался опровергнуть.** Пытался построить коллизию так, чтобы PAIRING её
пропустил — не вышло: любое отклонение от канонического EIP-2537
(паддинг, `x ≥ p`) даёт `false`, а не `true`.

**Если порт сделают наполовину правильно здесь.** Если при инлайне
`compressG2Unchecked` станет внутренней функцией, а `verify` где-то по дороге
заменят на «мы уже сжали, значит точка валидна» — связка рвётся и подделка
улик становится возможной. Инвариант, который надо перенести в порт дословно:
**результат `compress*Unchecked` не является доказательством ничего, пока те же
самые исходные 128/256 байт не прошли через PAIRING.**

---

## 3. Сверка с производящей стороной узла

Для каждого пункта §2 — место в узле, которое производит ровно этот формат.

| пункт §2 | место в узле | вердикт |
|---|---|---|
| уравнение `e(sig,−G2)·e(H,pk)==1` | `monorepo@3c4e02c/cryptography/src/bls12381/primitives/variant.rs:187-196` — `MinSig::verify` вызывает `G1::multi_pairing_check(&[hm], &[public], signature, &-G2::generator())` | совпадает |
| DST для PoP | `…/primitives/group.rs:386` `G1_PROOF_OF_POSSESSION = b"BLS_POP_BLS12381G1_XMD:SHA-256_SSWU_RO_POP_"` (43 B) | совпадает с `A:consensus.rs:27 BLS_POP_DST` |
| DST для голосов/улик | `…/group.rs:394` `G1_MESSAGE = b"BLS_SIG_BLS12381G1_XMD:SHA-256_SSWU_RO_POP_"` (43 B) | совпадает с `A:consensus.rs:817 BLS_SIG_DST` |
| `union_unique` | `monorepo@3c4e02c/utils/src/lib.rs:176-185` | см. 3.1 |
| сообщение PoP | `…/primitives/ops/mod.rs:109-115`: `hash_with_namespace(PROOF_OF_POSSESSION, namespace, &public.encode())`, `public.encode()` = 96 B сжатый G2 | совпадает: `A:consensus.rs:182` передаёт `compressed` (96 B) |
| сообщение голоса | `consensus/src/simplex/scheme/mod.rs:99-105`: `Notarize→proposal.encode()`, `Nullify→round.encode()`, `Finalize→proposal.encode()` | совпадает: `A:consensus.rs:1060,1072` передают `evidence.msg1/msg2` |
| namespace с суффиксом | `consensus/src/simplex/scheme/mod.rs:122-125` `_SEED/_NOTARIZE/_NULLIFY/_FINALIZE`, склейка через `union` (простая конкатенация) | совпадает: `A:consensus.rs:837-846` |
| базовый namespace | `B:crates/dpos/bls/src/lib.rs:111-116` `b"FLUENT_DPOS_V1_" ‖ chain_id.to_be_bytes()` = 23 B | совпадает: `A:consensus.rs:106-110` |
| hash-to-G1 | `…/primitives/group.rs:1333-1348` — `blst_hash_to_g1(msg, dst)` | совпадает побайтно, см. §4.3 |
| G1 128 B `pad‖x‖pad‖y` | `B:crates/dpos/bls/src/encoding.rs:40-51` | совпадает |
| G2 256 B `x.c0,x.c1,y.c0,y.c1` (перестановка половин относительно blst) | `B:crates/dpos/bls/src/encoding.rs:54-67`, слоты `[(0,48),(1,0),(2,144),(3,96)]` | совпадает |

### 3.1 Расхождение №1: LEB128 против однобайтовой длины

**Утверждение.** Узел кодирует длину namespace полным LEB128-варинтом, контракт —
одним байтом; сегодня они совпадают только потому, что все namespace короче 128
байт, и контракт ревертит вместо того, чтобы разойтись молча.

**Опора.**
- Узел: `utils/src/lib.rs:176-185` — `len_prefix.write(&mut buf)` для `usize`;
  `codec/src/types/primitives.rs:95-101` — `usize::write` делегирует в `UInt(u32)`;
  `codec/src/varint.rs:370-385` — классический LEB128 (пока `val >= 0x80`,
  пишем `low7|0x80`).
- Контракт: `orig.sol:81-82` — `if (ns.length >= 0x80) revert NamespaceTooLong();`
  и один байт длины.
- Фактические длины: PoP-namespace 23 B; с суффиксами 31 B (`_NULLIFY`) или 32 B
  (`_NOTARIZE`, `_FINALIZE`) — проверено на реальном выводе
  (`464c…5202` = 23 байта, `464c…5f4e4f544152495a45` = 32 байта).

**Уверенность.** `[KNOWN]`

**Чем пытался опровергнуть.** Проверил на контракте границу: `ns` длиной 127
байт проходит и даёт префикс `0x7f`, 128 — ревертит `NamespaceTooLong`.
То есть контракт fail-closed, молчаливого расхождения нет.

**Если порт сделают наполовину правильно здесь.** Если при инлайне «упростят»
и уберут проверку `>= 0x80` (мол, namespace всё равно короткий), то в день,
когда namespace станет длиннее 127 байт, узел и контракт начнут хешировать
РАЗНЫЕ сообщения — и это будет молча: подписи узла перестанут верифицироваться,
слэшинг перестанет срабатывать, а ошибки не будет ни одной. Проверку надо
перенести дословно.

### 3.2 Расхождение №2: DST-аргумент против DST-константы

**Утверждение.** Узел жёстко зашивает DST в типе (`MinSig::PROOF_OF_POSSESSION`
/ `MinSig::MESSAGE`), а контракт принимает DST аргументом от вызывающего.

**Опора.** `variant.rs:182-183` против `orig.sol:55` (параметр `bytes calldata dst`);
DST подставляет вызывающий — `A:consensus.rs:184` (`BLS_POP_DST`) и
`A:consensus.rs:1059,1071` (`BLS_SIG_DST`).

**Уверенность.** `[KNOWN]`

**Чем пытался опровергнуть.** Проверил, что верификатор не имеет никакой
собственной проверки DST кроме длины: с пустым DST он не ревертит, а возвращает
`false`; с DST длиной 255 — тоже `false`, а не revert. Значит защита целиком
на вызывающем.

**Если порт сделают наполовину правильно здесь.** При инлайне соблазн сделать
DST внутренней константой очень велик — и это правильно, ЕСЛИ обе константы
(`BLS_POP_…` и `BLS_SIG_…`) сохранятся раздельно. Если оставят одну — путь PoP
и путь улик схлопнутся в один домен, и подпись голоса станет пригодной как PoP.

### 3.3 Расхождение №3: узел проверяет подгруппу до пары, контракт — парой

**Утверждение.** Узел делает явный `validate()` / subgroup-check при декодировании,
контракт не делает никакого и полагается целиком на PAIRING.

**Опора.** `B:crates/dpos/bls/src/encoding.rs:45` (`point.validate(true)` —
infinity + subgroup) и `:58` (`point.validate()`); против `orig.sol` — ни одной
проверки точки кроме нулевой и длины.

**Уверенность.** `[KNOWN]`

**Если порт сделают наполовину правильно здесь.** Если при инлайне кто-то
решит, что «раз мы теперь внутри, PAIRING можно заменить на дешёвую проверку»,
исчезнет единственная проверка подгруппы во всей цепочке.

### 3.4 Расхождение №4: `_rejectInfinity` смотрит все байты, а не флаг

**Утверждение.** Контракт считает точку бесконечностью, только если **все**
128/256 байт нулевые; узел работает с z-cash-флагом `0xC0`.

**Опора.** `orig.sol:170-175` (цикл по всей длине, ранний `return` на первом
ненулевом байте); против `B:crates/dpos/bls/src/encoding.rs` тесты
`infinity_compressed_is_rejected` (байт 0 = `0xC0`).

**Уверенность.** `[KNOWN]`

**Замечание.** Это не расхождение по существу — EIP-2537 кодирует бесконечность
всеми нулями. Но проверка стоит `O(n)` в самом горячем месте (см. §7) и
переносится в порт как есть.

### 3.5 Что сверить не удалось

Кофакторную очистку в `MAP_FP_TO_G1` я проверил ЭМПИРИЧЕСКИ (совпадение с blst),
а не чтением реализации. И проверил на revm, а не на rWasm-предеплое Fluent —
см. §8.

---

## 4. Воспроизводимость

### 4.1 Что именно было запущено

**Утверждение.** Развёрнутый верификатор запускался по-настоящему: контракт
задеплоен на `anvil --hardfork prague`, векторы произведены настоящим кодом узла.

**Опора.**
- `anvil 1.6.0-v1.7.0`, `--hardfork prague`, порт 8599. Предеплой `0x10`
  отвечает на 64 нулевых байта осмысленной точкой — EIP-2537 у него есть.
- Верификатор задеплоен из `bytecode.object` артефакта по адресу
  `0x5fbdb2315678afecb367f032d93f642f64180aa3`, `status=0x1`.
- Векторы: отдельный крейт `scratchpad/popgen`, зависящий по пути от
  `B:crates/dpos/bls` и от того же `commonware v2026.4.0`. Он вызывает
  `ValidatorBlsKeypair::generate`, `pop::sign_pop`, `pop::verify_pop`,
  `encoding::*`, `ops::sign_message::<MinSig>`, `ops::verify_message::<MinSig>` —
  то есть НАСТОЯЩИЙ производитель, а не переписанный.
  Узел сам принимает свои PoP и подпись (`verify_pop(...).expect`,
  `verify_message(...).expect` не паникуют).

**Уверенность.** `[KNOWN]`

### 4.2 Валидный вход — `true`, испорченный на один байт — `false`

Все строки ниже — фактический вывод `cast call` к развёрнутому контракту.

```
=== A. путь PoP (DST = BLS_POP_…, namespace без суффикса) ===
verify(PoP) valid                      -> true
verify(PoP) POP128    последний байт ^1 -> false
verify(PoP) PK256     последний байт ^1 -> false
verify(PoP) PK96(msg) последний байт ^1 -> false
verify(PoP) NAMESPACE последний байт ^1 -> false
verify(PoP) POP_DST   последний байт ^1 -> false

=== B. путь голоса/улик (DST = BLS_SIG_…, namespace ‖ _NOTARIZE) ===
verify(vote) valid                      -> true
verify(vote) SIG128 последний байт ^1   -> false
verify(vote) MSG    последний байт ^1   -> false
verify(vote) NS_NOT последний байт ^1   -> false
```

Ключевые байты вектора PoP (seed 1, chain_id 20994):
```
NAMESPACE 464c55454e545f44504f535f56315f0000000000005202          (23 B)
PK96      85bcdb78…edb63bb9                                        (96 B)
POP48     b12ec227…99f20b8                                         (48 B)
```

Таксономия ревертов проверена там же (§2.8) — все шесть селекторов получены
фактическими вызовами, не выведены из кода.

**Уверенность.** `[KNOWN]`

**Чем пытался опровергнуть.** Портил не только подпись, но и ключ, и сообщение,
и namespace, и DST — по одному байту. Ни один не дал `true`. Отдельно проверил,
что «плохая точка» даёт `false`, а не `true`: ненулевой паддинг и `x ≥ p`
отвергаются PAIRING.

### 4.3 hash-to-G1 совпал побайтно на всём конформанс-корпусе

**Утверждение.** Внутренняя `_hashToG1` контракта даёт ровно тот же G1-пойнт,
что `blst_hash_to_g1` в узле, на всех шести пинованных векторах — оба DST,
все четыре субъекта, пустое сообщение, 344-байтное сообщение, chain_id 0 и
`u64::MAX`.

**Опора.** Корпус получен запуском узлового генератора:
`cargo test -p fluentbase-bls --test hash_to_g1_conformance -- --ignored
print_corpus --nocapture` (вывод с `EXPECTED_H` и преимеджами). Затем собран и
задеплоен `test/bls/BLS12381VerifierHarness.sol` из C (наследник верификатора,
открывающий `_hashToG1`) по адресу `0x8a791620dd6260079bf849dc5567adc3f2fdc318`
и прогнан:

```
pop_main_pk96            MATCH
pop_chain0_empty         MATCH
notarize_main_proposal   MATCH
nullify_main_round       MATCH
finalize_chainmax_long   MATCH
sig_chain0_short         MATCH
ALL 6 RECIPES MATCH
```

**Уверенность.** `[KNOWN]`

**Чем пытался опровергнуть.** Корпус специально содержит краевые случаи
(пустое сообщение, chain_id = `u64::MAX`, 32-байтный namespace, 344-байтное
сообщение — четыре блока SHA-256 в `msgP`). Ни один не разошёлся.

**Если порт сделают наполовину правильно здесь.** Этот корпус — единственный
существующий детектор дрейфа хеш-конвейера. Он должен переехать в порт целиком;
`solidity-contracts/test/bls/BlsHashToG1Conformance.t.sol` уже содержит все шесть
`EXPECTED_H` и оба вектора `VERIFY_EXPECTED` (проверено grep'ом по префиксам:
6 из 6 и 2 из 2).

### 4.4 compress* совпал с узловым сжатием

```
compressG1(POP128)   == узловой POP48    MATCH
compressG2(PK256)    == узловой PK96     MATCH
compressG1(H128)     == узловой H48      MATCH
compressG2(PK2_256)  == узловой PK2_96   MATCH
unionUnique(ns,pk96) == union_unique узла MATCH
```
**Уверенность.** `[KNOWN]`

### 4.5 Тот же конвейер против реализации, которая реально стоит на Fluent

Первая редакция этого документа фиксировала здесь дыру: всё гонялось на
anvil (revm + **blst**), а Fluent считает EIP-2537 своими rWasm-предеплоями.
Дыра закрыта отдельным прогоном (2026-09-08).

**Утверждение.** Реализация, которая стоит на Fluent, даёт побайтно те же
результаты, что blst, на всех входах этого документа.

**Опора.** Четыре независимых прогона.

1. **BLS12-381 (адреса `0x0b`, `0x0f`, `0x10`).** Скретч-крейт
   `scratchpad/arkcheck` объявляет `revm-precompile` из
   `fluentlabs-xyz/revm-rwasm@86741222` (тот же rev, что в `contracts/Cargo.lock`)
   с `default-features = false` — то есть ровно так, как это делают
   `contracts/bls12381` и `contracts/modexp`, и с выключенной фичей `blst`.
   Проверено `cargo tree`: `blst` в графе **отсутствует**, `ark-bls12-381 v0.5.0`
   присутствует. Крейт зовёт ТЕ ЖЕ функции, что и `contracts/bls12381/src/lib.rs`:
   `map_fp_to_g1`, `g1_add`, `pairing`, — и воспроизводит конвейер `_hashToG1`
   дословно по §2.4. Вывод:

   ```
   == A. hash-to-G1 через arkworks против эталона blst ==
   pop_main_pk96            MATCH
   pop_chain0_empty         MATCH
   notarize_main_proposal   MATCH
   nullify_main_round       MATCH
   finalize_chainmax_long   MATCH
   sig_chain0_short         MATCH

   == B. verify через arkworks (те же векторы, что на anvil) ==
   verify(PoP) valid                            -> true  OK
   verify(PoP) POP128 последний байт ^1         -> false OK
   verify(PoP) PK256 последний байт ^1          -> false OK
   verify(PoP) PK96(msg) последний байт ^1      -> false OK
   verify(PoP) NAMESPACE последний байт ^1      -> false OK
   verify(PoP) POP_DST последний байт ^1        -> false OK
   verify(vote) valid                           -> true  OK
   verify(vote) SIG128 последний байт ^1        -> false OK

   == C. краевые случаи, которые PAIRING обязан отвергнуть ==
   pk: ненулевой 16-байтный паддинг             -> false OK
   sig: x >= p (неканоничный Fp)                -> false OK
   ```

   Эталон в блоке A — `EXPECTED_H` из
   `B:crates/dpos/bls/tests/hash_to_g1_conformance.rs`, посчитанный **blst**
   (`blst_hash_to_g1`). Совпадение доказывает сразу две вещи: конвейер
   воспроизведён верно И arkworks согласен с blst на этих входах. Одно без
   другого совпасть не могло бы.

2. **MODEXP (`0x05`).** `contracts/modexp/src/lib.rs:15` зовёт
   `revm_precompile::modexp::osaka_run` — та же функция вызывается в `arkcheck`.
   Штатные тесты крейта: `cargo test -p fluentbase-contracts-modexp` — **6 passed**.

3. **rWasm-обёртка BLS.** `cargo test -p fluentbase-contracts-bls12381` —
   **18 passed** на стандартных векторах EIP-2537 из `testcases/*.json`,
   включая все `fail-*`. Это покрывает то, что `arkcheck` обходит: проверки длины
   и газа, `sdk.bytes_input()`/`sdk.write`.

4. **SHA-256 (`0x02`).** `contracts/sha256` зовёт
   `fluentbase_crypto::crypto_sha256` — это **своя** реализация
   (`B:crates/crypto/src/sha256.rs`, 63 строки, раунды через
   `CryptoRuntime::sha256_compress`/`sha256_extend`), а не обёртка над `sha2`, и
   у неё в крейте ровно один тест («hello world»). Скретч-крейт
   `scratchpad/shacheck` сверил её с `sha2` на **551 входе**: все длины `0..=520`
   (обе ветки паддинга — `rem <= 55` и `rem > 55`, плюс переходы через границу
   блока) и все пять фактических `sha256`-вызовов конвейера для каждого из шести
   рецептов. Расхождений: **0**.

**Уверенность.** `[KNOWN]`

**Чем пытался опровергнуть.** Специально проверил графом зависимостей, что
`blst` не подтянулся через унификацию фич — иначе прогон сравнивал бы blst с
blst и ничего не доказывал. `cargo tree -i blst` → «nothing to print».
Отдельно взял для SHA-256 не только реальные входы, но и сплошной диапазон длин,
потому что ошибка в ручном паддинге проявляется на конкретных остатках, а на
«боевых» длинах могла бы не всплыть.

**Что осталось за границей и здесь.** Все четыре прогона исполнялись на
x86_64-нативной сборке того же исходного кода, а не на rwasm-скомпилированном
модуле внутри рантайма Fluent. То есть проверено «реализация та же и она
согласна с blst», но не «rwasm-компиляция этой реализации ведёт себя так же».
Это вопрос к компилятору/рантайму, а не к криптографии; в этом проходе он не
закрывался.

---

## 5. Что ломается при удалении внешнего адреса

Список составлен сам, по обоим деревьям плюс питон-стенд плюс байткод
развёрнутых на стенде солидити-контрактов. Список пользователя оказался неполным.

### 5.1 Ломается МОЛЧА (компилируется, но зовёт не то)

Пять мест.

1. **`B:devnet/local-dpos-smoke/genesis-bootstrap/src/bootstrap.rs:90-108`** —
   своя копия интерфейса в `sol!`, 17 аргументов, включая `address blsVerifier`
   пятнадцатым. Удаление поля в A меняет селектор `initialize`
   **с `0xdfa8efb0` на `0xfecaf0f1`** (посчитано `cast sig` для обеих сигнатур),
   а этот `sol!` продолжает компилироваться и продолжает слать старый селектор.
   Результат — `initialize` не найден, генезис-бутстрап падает на рантайме, не на
   сборке. Само заполнение — `:354 blsVerifier: BLS_VERIFIER_ADDR`.
   `[KNOWN]`
2. **`B:devnet/local-dpos-smoke/dpos_harness/stack/production_path.py:210-211`** —
   сигнатура `INITIALIZE_SIG` захардкожена строкой; `:719` подставляет
   `self.verifier` пятнадцатым аргументом. Питон вообще ничего не проверяет:
   `cast send` уйдёт со старым селектором. `[KNOWN]`
3. **`A:contracts/staking/src/tests.rs:1107`** — пин `(SIG_INITIALIZE, 0xdfa8efb0)`.
   Единственное место, которое ЗАМЕТИТ изменение — тест упадёт. Это не поломка, а
   страховка; её надо обновить осознанно. `[KNOWN]`
4. **`A:contracts/staking/src/tests.rs:214-255`** — фабрика `InitializeCommand` в
   харнессе задаёт `bls_verifier`. При удалении поля не скомпилируется (не молча),
   но 12+ тестов, зовущих `initialize`, зависят от него. `[KNOWN]`
5. **`B:devnet/local-dpos-smoke/genesis-bootstrap/tests/bootstrap_smoke.rs:126,327`** —
   утверждают наличие кода по `BLS_VERIFIER_ADDR`; после инлайна предеплой станет
   лишним, и эти утверждения станут неверными по смыслу, оставаясь зелёными,
   пока предеплой продолжают ставить. `[KNOWN]`

### 5.2 Ломается ГРОМКО (не скомпилируется / явно упадёт)

6. `A:contracts/staking/src/types.rs:30` — само поле.
7. `A:contracts/staking/src/config.rs:77-81` — запись поля при инициализации.
8. `A:contracts/staking/src/config.rs:129-135` — событие `BlsVerifierChanged` при
   инициализации.
9. `A:contracts/staking/src/config.rs:713-723` — публичный `getBlsVerifier()`
   (`0xc6b904ad`).
10. `A:contracts/staking/src/config.rs:726-742` — публичный `setBlsVerifier(address)`
    (`0x466ae541`), губернаторский.
11. `A:contracts/staking/src/consts.rs:125,127` — `SIG_GET_BLS_VERIFIER`,
    `SIG_SET_BLS_VERIFIER`.
12. `A:contracts/staking/src/consts.rs:214,216,229` — `SIG_BLS_COMPRESS_G2_UNCHECKED`,
    `SIG_BLS_VERIFY`, `SIG_BLS_COMPRESS_G1_UNCHECKED` (селекторы внешних вызовов).
13. `A:contracts/staking/src/consts.rs:309` — `ERR_BLS_VERIFIER_NOT_CONFIGURED`.
14. `A:contracts/staking/src/lib.rs:71-72` — диспетчер обоих хендлеров.
15. `A:contracts/staking/src/events.rs:97` — событие `BlsVerifierChanged`.
16. `A:contracts/staking/src/storage.rs:35` — слот `bls_verifier: StorageAddress`
    в `ChainConfigStorage`; его удаление **сдвигает слоты**
    `min_undelegate_blocks`, `blend_reserve`, `min_verdict_due_blocks`,
    `exclusion_backoff_cap` и далее. Ничто вне крейта раскладку не читает
    (`ChainConfigStorage` упоминается только в `storage.rs`), и деплоев нет —
    но об этом надо знать.
17. `A:contracts/staking/src/consensus.rs:152-155, 156-165, 178-193` — путь PoP:
    получение адреса, `compressG2Unchecked`, `verify`.
18. `A:contracts/staking/src/consensus.rs:1000-1002, 1005-1010, 1036-1048,
    1053-1075` — путь улик: адрес, `compressG2Unchecked`, два `compressG1Unchecked`,
    два `verify`.
19. `A:contracts/staking/src/tests.rs:1090` (`(SIG_SET_BLS_VERIFIER, "blsVerifier")`),
    `:1135` (`(SIG_SET_BLS_VERIFIER, 0x466ae541)`), `:1700`, `:1753`, `:1810`,
    `:1842` — тесты губернаторских сеттеров/геттеров.
20. `A:contracts/staking/src/tests.rs:439-453` — заглушки трёх селекторов
    (`mock_external_return`).
21. `A:contracts/staking/src/tests.rs:750-766, 802-804, 853-870, 2056-2065,
    2317, 5516, 5560` — ещё семь мест с собственными обработчиками вызова
    верификатора.

### 5.3 Стенд и артефакты

22. `B:devnet/local-dpos-smoke/genesis-bootstrap/src/artifacts.rs:28,58` — поле
    `bls_verifier` и загрузка `BLS12381Verifier.json`. Если удалить артефакт,
    падает на старте.
23. `B:devnet/local-dpos-smoke/genesis-bootstrap/src/bootstrap.rs:22` —
    `BLS_VERIFIER_ADDR = 0x…5208`; `:200-206` деплой рантайм-байткода;
    `:218` включение в `wrap_evm_predeploys`; `:432` включение в `snapshot`.
24. `B:devnet/local-dpos-smoke/dpos_harness/stack/production_path.py:164` —
    `VERIFIER_CONTRACT = "contracts/libraries/BLS12381Verifier.sol:BLS12381Verifier"`
    и `:506` `forge create` этого пути в соседнем репозитории C.
    **Отдельная хрупкость, которая существует УЖЕ СЕЙЧАС:** файл есть только на
    ветках `djadjka/dpos-audit-fixes` / `djadjka/bls-staking`; текущий чекаут C
    стоит на `djadjka/rollup-audit`, где файла нет, и `forge create` по этому пути
    не найдёт контракт. `[KNOWN]`
25. `B:devnet/local-dpos-smoke/contracts/BLS12381Verifier.json` — сам артефакт,
    и `B:devnet/local-dpos-smoke/Makefile:22` — строка `regen-contracts`,
    копирующая его из C.
26. `B:devnet/local-dpos-smoke/dpos_harness/tests/test_prod_substrate.py:759-770` —
    тест, утверждающий, что `setBlsVerifier` НЕ вызывается в bring-up; после
    инлайна он останется зелёным, но потеряет смысл.
27. `B:devnet/local-dpos-smoke/dpos_harness/tests/test_prod_substrate.py:968-969` —
    губернаторский тест использует строку `"setBlsVerifier"` как описание
    предложения, которое должно быть Defeated. Строка станет отсылкой к
    несуществующей функции.
28. `B:devnet/local-dpos-smoke/dpos_harness/cases/smoke/asserts_prod.py:195` —
    комментарий про «keys after setBlsVerifier».

### 5.4 Байткод развёрнутых на стенде солидити-контрактов — проверено, чисто

**Утверждение.** Ни один другой вендоренный солидити-артефакт стенда не содержит
ни селекторов верификатора, ни его адреса.

**Опора.** Прогон по всем семи `.json` в `devnet/local-dpos-smoke/contracts/`
с поиском `8bf26133`, `a5d2dd22`, `8f498050`, `654f3ba0`, `c6b904ad`,
`466ae541`, `dfa8efb0` и литерала `0x…5208` в `deployedBytecode.object`:
попадания только в самом `BLS12381Verifier.json`. `StakingPool.json`,
`FluentGovernance.json`, `MockBlendToken.json`, `MockRollup.json`,
`PrevRandaoProbe.json`, `GasBurner.json` — чисто.

**Уверенность.** `[KNOWN]`

### 5.5 Узел (`crates/`, `bins/`) — чисто

**Утверждение.** Ни один крейт узла не читает адрес верификатора и не зовёт его.

**Опора.** `grep -rn "blsVerifier\|bls_verifier\|BLS12381Verifier" crates/ bins/
--include='*.rs'` даёт ровно две строки, обе — комментарии:
`crates/dpos/bls/tests/hash_to_g1_conformance.rs:3` и
`crates/dpos/consensus/src/scheme.rs:24`.

**Уверенность.** `[KNOWN]`

### 5.6 Документация (обязательна к правке по CLAUDE.md)

Пять секций архитектуры упоминают верификатор и должны быть обновлены в том же
изменении: `.claude/dpos_architecture/{01_system_map, 06_staking_layer,
07_slashing_end_to_end, 11_l1_contracts_solidity_contracts,
13_invariants_gotchas_rules}.md`. `[KNOWN]` (список от `grep -rln`).

### 5.7 Итог

**Молча ломающихся мест снаружи контракта: два.**
`genesis-bootstrap/src/bootstrap.rs` (своя копия `sol!`) и
`dpos_harness/stack/production_path.py` (захардкоженная строка сигнатуры).
Остальные 26 позиций либо не скомпилируются, либо упадут явно, либо это
документация/артефакты.

**Уверенность.** `[KNOWN]`

**Чем пытался опровергнуть.** Искал третий молчаливый путь: проверил байткод
всех вендоренных солидити-артефактов стенда (чисто, §5.4), весь `crates/`+`bins/`
(чисто, §5.5), и все места в B, где встречаются селекторы `getBlsVerifier` /
`setBlsVerifier` / `initialize` — их всего пять файлов, четыре из которых
питоновские тексты и один сам артефакт.

**Если порт сделают наполовину правильно здесь.** Обновят `A`, обновят питон,
забудут `sol!` в `bootstrap.rs` — и генезис-стенды (не `production_path`, а
`docker-compose.dpos.yml` / `.cert-catchup.yml`) перестанут подниматься с
непонятной ошибкой «нет такого селектора», а не с «вы сменили ABI».

---

## 6. Что станет с тестами

### 6.1 Заглушки сегодня врут

**Утверждение.** Дефолтная заглушка `verify` в A возвращает `true` всегда, а
`compressG2Unchecked` — арифметический узор от первого байта; поэтому ни один
тест на дефолтной заглушке не проверяет ни криптографию, ни связывание личности.

**Опора.** `A:contracts/staking/src/tests.rs:439-453`:
```
SIG_BLS_COMPRESS_G2_UNCHECKED => compressed = uncompressed[0].wrapping_add(0x22);
                                 вернуть 96 байт этого значения
SIG_BLS_COMPRESS_G1_UNCHECKED => вернуть первые 48 байт входа
SIG_BLS_VERIFY               => вернуть true
```
Комментарий на `:432-436` это прямо описывает: ключ из `0x11`-байтов «владеет»
`0x33`-ключом.

**Уверенность.** `[KNOWN]`

### 6.2 Какие тесты после инлайна теряют смысл

1. **`derived_selectors_match_independent_hex_pins`** (`tests.rs:1104-1140`) в
   части `(SIG_SET_BLS_VERIFIER, 0x466ae541)` и `(SIG_INITIALIZE, 0xdfa8efb0)` —
   первая строка исчезает, вторая меняется на `0xfecaf0f1`.
2. **Тесты губернаторских сеттеров** `tests.rs:1090, 1700, 1753, 1810, 1842` в
   части `blsVerifier` — предмета больше нет.
3. **`solidity_bytes_outputs_and_event_match_cast_vectors`** (`tests.rs:840-880`)
   в части, пинующей ABI-кодирование внешних вызовов `0xa5d2dd22` / `0x8bf26133`
   — после инлайна внешних вызовов нет, кодировать нечего.
4. **`tests.rs:800-804`** — утверждение «ровно два внешних вызова, первый
   compressG2, второй verify» становится «ноль внешних вызовов».
5. **`record_verify_namespaces`** (`tests.rs:5539-5580`) — весь механизм
   перехвата внешнего вызова, чтобы подсмотреть namespace. Это самый ценный из
   перечисленных тестов (`each_slash_route_verifies_under_the_domain_its_kinds_name`
   — единственное, что отличает легальную пару notarize+nullify от слэшабельной),
   и его надо не выбросить, а переписать: после инлайна namespace надо будет
   наблюдать иначе.

### 6.3 Какие негативные ветки недостижимы СЕГОДНЯ

**Утверждение.** Из трёх названных веток две не покрыты ни одним тестом, третья
покрыта.

**Опора.** Подсчёт по всему крейту A:

| ветка | всего упоминаний | из них в `tests.rs` |
|---|---|---|
| `ERR_INVALID_PROOF_OF_POSSESSION` (`consensus.rs:192`) | 2 | **0** |
| `ERR_BLS_VERIFIER_NOT_CONFIGURED` (`consensus.rs:155, 1002`) | 3 | **0** |
| `ERR_EQUIVOCATION_SIGNATURE_INVALID` | — | есть (`tests.rs:5653`, `:6303`) |
| `ERR_EQUIVOCATION_KEY_NOT_REGISTERED` | 3 | 1 (`tests.rs:6282`) |

То есть:
- **`InvalidProofOfPossession` — недостижим.** Дефолтная заглушка отвечает `true`,
  и ни один тест не ставит обработчик, отвечающий `false` на пути PoP.
  (Обработчик `record_transfers(..., signatures_valid=false)` на `tests.rs:5516`
  возвращает `false`, но применяется на пути УЛИК, где ошибка другая.)
- **`BlsVerifierNotConfigured` — недостижим.** Харнесс ставит нулевой верификатор
  только когда `validator_count == 0` (`tests.rs:247-251`), то есть когда ни одна
  проверка ключа не запускается; `initializer.rs` нулевой адрес не отвергает
  (grep по `bls_verifier` в `initializer.rs` — пусто).
- **Отказ внешнего вызова** (`external_call` вернул не-Ok, `consensus.rs:65-68`)
  — заглушки всегда отвечают `ExitCode::Ok` либо `MalformedBuiltinParams` для
  неизвестного селектора; ветка «верификатор ревертнул `InvalidPointLength`»
  не проходится ни разу.

**Уверенность.** `[KNOWN]`

**Чем пытался опровергнуть.** Не поверил счётчику: прочёл оба кастомных
обработчика (`record_transfers`, `record_verify_namespaces`) целиком и убедился,
что первый бьёт по пути улик, второй — тоже (он отвечает `false` на
неразрешённый namespace, что даёт `ERR_EQUIVOCATION_SIGNATURE_INVALID`, а не
`InvalidProofOfPossession`).

### 6.4 Чем заменить заглушку

**Ответ: честным хостовым стабом, умеющим вернуть false. Не e2e.**

Обоснование, а не перечисление:

Инлайн УБИРАЕТ внешний вызов. Значит подменять станет нечего: `verify` станет
внутренней функцией, которая пойдёт в предеплои `0x02/0x05/0x0b/0x0f/0x10`.
Юнит-харнесс A (`fluentbase_testing::TestingContextImpl`) эти предеплои не
исполняет — он перехватывает `sdk.call`. Поэтому после инлайна точка подмены
сдвигается на уровень ниже: **не «верификатор ответил true», а «предеплой
`0x0f` ответил 1»**. Стаб должен стоять именно там и уметь отвечать «0».

Почему не e2e через настоящий предеплой:
- e2e не даёт негативных веток. Чтобы `verify` вернула `false` в e2e, нужно
  сгенерировать заведомо неверную подпись — это делается, но получить оттуда
  ветку «MODEXP вернул не 48 байт» (`PrecompileFailed`) невозможно вообще:
  корректный предеплой так себя не ведёт.
- e2e стоит дорого (см. §7: одна `verify` — 165 789 газа исполнения; тестов
  слэшинга в A порядка нескольких десятков).
- e2e уже есть в другом месте и в лучшем виде: конформанс-корпус §4.3 плюс
  `solidity-contracts/test/bls/*.t.sol`. Дублировать его в rWasm-юнит-тестах
  бессмысленно.

Что стаб обязан уметь, чтобы не повторить сегодняшнюю ложь:
1. отвечать `1` и `0` на `0x0f` (PAIRING) по решению теста — это включает
   `InvalidProofOfPossession` и `ERR_EQUIVOCATION_SIGNATURE_INVALID`;
2. отвечать неверной длиной на `0x05`/`0x10`/`0x0b` — это включает
   `PrecompileFailed`;
3. **записывать calldata каждого вызова**, чтобы сохранился аналог
   `record_verify_namespaces`: namespace теперь виден только внутри `msgP`,
   который уходит в `sha256`. Без этого тест
   `each_slash_route_verifies_under_the_domain_its_kinds_name` — единственный,
   отличающий легальную пару голосов от слэшабельной, — умрёт молча;
4. считать `sha256` честно (иначе п.3 не собрать).

Плюс к стабу — одна вещь, которую стаб не заменит: **конформанс-корпус §4.3
должен переехать в порт** как отдельный тест, сравнивающий байты, а не как
свойство. Сегодня он живёт в двух местах (Rust `EXPECTED_H` и рукописное
зеркало в C), и при инлайне зеркало в C осиротеет.

**Уверенность.** `[KNOWN]` в части фактов, вывод — инженерное суждение.

**Если порт сделают наполовину правильно здесь.** Оставят стаб, который отвечает
`1` на `0x0f` всегда, — и получат ровно ту же слепоту, что сегодня, только
этажом ниже и менее заметную: сейчас хотя бы видно, что `SIG_BLS_VERIFY =>
true`, а «PAIRING всегда 1» выглядит как техническая деталь.

### 6.5 Отдельно: узловой конформанс-тест сам себя не проверяет

**Утверждение.** `B:crates/dpos/bls/tests/hash_to_g1_conformance.rs` НЕ проверяет
солидити — он сравнивает узел с константами, которые сам же и сгенерировал.

**Опора.** `conformance_corpus_matches_committed_constants` (`:229-247`)
сравнивает `hash_eip2537(r)` (тот же `ops::hash`) с `EXPECTED_H`; регенератор
`print_corpus` (`:291+`) печатает `EXPECTED_H` из того же `ops::hash`. Заголовок
файла (`:11-13`) сам говорит, что зеркало в
`solidity-contracts/test/bls/BlsHashToG1Conformance.t.sol` переносится **вручную**.

**Уверенность.** `[KNOWN]`

**Что это значит.** Rust-тест — детектор дрейфа commonware/blst, и только.
Кросс-языковую проверку делает только солидити-зеркало. Я эту проверку выполнил
фактически (§4.3) — и она прошла. Но в CI её сегодня выполняет тест из
репозитория C, которого нет в дереве B; после инлайна за неё будет отвечать
порт, и если её не перенести, ничто не заметит расхождения.

---

## 7. Замеры топлива (ДО правки)

Замер, не оценка. `anvil --hardfork prague`, транзакции к развёрнутому
верификатору `0x5fbdb…0aa3`, `gasUsed` из квитанции. «Исполнение» = `gasUsed`
минус `21000 + 16·ненулевых + 4·нулевых` байт calldata.

| вызов | tx `gasUsed` | intrinsic+calldata | исполнение |
|---|---|---|---|
| `verify` (PoP, валидный) | **195 897** | 30 108 | **165 789** |
| `verify` (голос, msg 35 B) | 195 110 | 29 368 | 165 742 |
| `compressG1Unchecked` (128 B) | **57 139** | 22 996 | **34 143** |
| `compressG2Unchecked` (256 B) | **85 472** | 24 660 | **60 812** |
| `unionUnique` (23+96 B) | 27 230 | 23 492 | 3 738 |

Проверочный замер предеплоя: `PAIRING` на 2 пары — `gasUsed` 133 860,
исполнение ровно **102 900**, что совпадает со спецификацией EIP-2537
(`32600·2 + 37700`). То есть из 165 789 газа `verify` 102 900 (62 %) —
неснижаемая пара; ~11 000 — два `MAP_FP_TO_G1`; 375 — `G1ADD`; остальное
(~51 500) — SHA-256, MODEXP и накладные Solidity.

### Пересчёт на пути контракта

Стейкинг делает (посчитано по `A:consensus.rs`):

| путь | вызовы | сумма tx-стоимости этих вызовов | сумма исполнения |
|---|---|---|---|
| `registerConsensusKeys` / `registerValidator` | 1 × compressG2 + 1 × verify | 281 369 | **226 601** |
| `slashEquivocation*` | 1 × compressG2 + 2 × compressG1 + 2 × verify | 591 618 | **460 630** |

### Чего этот замер НЕ содержит

**Это стоимость EVM-стороны и только её.** Он не включает:
- газ rWasm-стейкинга на ABI-кодирование семи вызовов и декодирование ответов;
- накладные семи `CALL` (первый — холодный доступ к адресу верификатора, 2600, остальные тёплые, по 100);
- разницу между revm-предеплоями и rWasm-предеплоями Fluent — на Fluent те же
  `0x0f`/`0x10` считает `contracts/bls12381`, и его тарификация может отличаться.

**Замерить стоимость самих `registerConsensusKeys` и `slashEquivocation*`
целиком мне нечем в этом проходе:** для этого нужен поднятый стенд
`devnet/local-dpos-smoke` с rWasm-стейкингом, а не anvil. Это не сделано, и я не
выдаю EVM-часть за полную стоимость. Что даёт эта таблица — корректный
**базис для сравнения после инлайна**: убрать надо будет ровно эти 226 601 /
460 630 газа исполнения плюс накладные вызовов, а добавить — то же самое
исполнение внутри стейкинга.

**Уверенность.** `[KNOWN]` для таблиц, `[LIKELY]` для утверждения, что
rWasm-предеплои тарифицируются иначе.

### Побочное наблюдение

`cast estimate` на `verify` вернул **99 341** при фактических **195 897**.
Для `compressG1`/`compressG2`/`unionUnique` оценка совпала с фактом до газа.
То есть `eth_estimateGas` у anvil 1.6.0 недооценивает вызовы с
EIP-2537-предеплоями почти вдвое. Для этой задачи неважно, но если кто-то будет
мерить газ через `estimate`, он получит цифру вдвое меньше настоящей.
`[KNOWN]`

---

## 8. Открытое

### 8.1 Две реализации BLS12-381 в цепочке — установлено и сверено

Изначально это был самый слабый пункт документа. Он закрыт (§4.5), но факт
остаётся и его надо знать при порте.

**Утверждение.** В цепочке участвуют ДВЕ РАЗНЫЕ реализации BLS12-381.

- Узел подписывает через **blst** (`monorepo@3c4e02c/cryptography/src/bls12381/
  primitives/group.rs:1333-1348` — `blst_hash_to_g1`).
- anvil/reth считают пары тоже через **blst**.
- Fluent за адресами `0x0b…0x11` считает через **arkworks**.

**Опора.** `B:contracts/bls12381/Cargo.toml` объявляет
`revm-precompile = { workspace = true }`, а `B:Cargo.toml:206` задаёт этой
зависимости `default-features = false` — фича `blst` выключена. Выбор бэкенда в
revm — `cfg_if` в `revm-rwasm@8674122/crates/precompile/src/bls12_381.rs:8-14`:
`if #[cfg(feature = "blst")] { blst } else { arkworks }`. Подтверждено по
собранному артефакту: в
`target/contracts/wasm32-unknown-unknown/release/fluentbase_contracts_bls12381.wasm`
`strings | grep -c blst` = **0**, arkworks-путей 7 (`ark-ec-0.5.0`,
`ark-ff-0.5.0`, `ark-poly-0.5.0`).

**Уверенность.** `[KNOWN]`

**Статус.** Согласие двух реализаций на всех входах этого документа проверено —
см. §4.5, шесть векторов hash-to-G1 и оба вектора `verify` совпали побайтно.

**Что при этом всё равно остаётся.** Согласие проверено на конкретном наборе
входов, а не доказано вообще. Два независимых пути — это постоянный источник
риска: обновление `revm-rwasm` может сменить бэкенд или его версию, и заметить
это будет нечем, потому что в дереве B нет ни одного теста, который сравнивал бы
Fluent-реализацию с blst-эталоном узла. Конформанс-корпус
(`crates/dpos/bls/tests/hash_to_g1_conformance.rs`) сегодня сравнивает узел
только с самим собой (§6.5). Прогон §4.5 сделан скретч-крейтом и в дерево не
попал.

**Побочно.** Security-note в самом `orig.sol:141-153` про очистку кофактора
ссылается на `blst src/map_to_g1.c` и на gnark-crypto в go-ethereum. **Ни то, ни
другое на Fluent не исполняется.** Комментарий не улика, но он показывает, что
при написании контракта эту реализацию не рассматривали. Фактически кофактор
arkworks чистит — иначе §4.5 не совпал бы с blst.

### 8.1a Инлайн самой криптографии в стейкинг — отдельно рассмотрено и отклонено

Вопрос «а не втащить ли `contracts/bls12381` в стейкинг вместо вызова предеплоя»
разобран и закрыт отрицательно; фактура:

- `contracts/bls12381` **и есть** предеплой: `crate-type = ["cdylib"]`,
  единственная публичная точка — `main_entry` + `system_entrypoint!`
  (`contracts/bls12381/src/lib.rs:134,152`). Как библиотеку его подключить
  нельзя; подключать пришлось бы `revm-precompile` напрямую.
- Стоимость по размеру (измерено): каждый контракт, тянущий `revm-precompile`,
  весит ~890 КБ wasm (`blake2f` 888 747, `ripemd160` 888 318, `kzg` 888 235,
  `bls12381` 905 834), тогда как `sha256` — 71 927, `identity` — 68 029.
  Стейкинг сегодня 411 835 wasm / 2 831 192 rwasm; `bls12381.rwasm` — 5 678 137.
- Выигрыша по корректности нет: это был бы **тот же самый** arkworks-код.
- Появился бы второй экземпляр той же криптографии на одной цепи, обязанный
  совпадать побайтно с посаженным `0x0f` навсегда; расхождение версий проявилось
  бы как молча несработавший слэшинг.
- Выигрыша по доверию тоже нет: сам модуль стейкинга ставится тем же
  `runtime-upgrade` (`devnet/local-dpos-smoke/dpos_harness/stack/production_path.py`
  — `runtime-upgrade install-local`). Кто может подменить `0x0f`, тот может
  подменить и стейкинг вместе с вшитой в него парой.
- По газу — неизвестно и скорее хуже: предеплой тарифицируется фиксированным
  расписанием EIP-2537 (102 900 на две пары, замерено), а вшитый код мерился бы
  топливом rWasm по фактическим инструкциям arkworks-пары.

### 8.2 Кофакторная очистка проверена эмпирически, а не по коду

Я не читал ни реализацию `MAP_FP_TO_G1` в revm/blst, ни в `contracts/bls12381`.
Вывод «кофактор очищается» сделан из того, что результат совпал с `blst_hash_to_g1`
на шести векторах. Этого достаточно для revm; для rWasm — нет (см. 8.1).

### 8.3 Точный текст исходника восстановлен, но не доказан побайтно из артефакта

Я доказал две вещи по отдельности: (а) файл из C имеет keccak, записанный в
метаданных артефакта; (б) AST артефакта позволяет собрать семантически
идентичный файл. Я НЕ восстанавливал оригинальные пробелы и комментарии из
`src`-смещений AST — этого не требовалось, потому что файл нашёлся. Если бы он
не нашёлся, порт всё равно был бы возможен из AST, но комментарии (включая
security-note про кофактор) были бы утеряны.

### 8.4 Не проверено: `evidence.msg1` == `proposal.encode()` побайтно

Я установил, что commonware подписывает `proposal.encode()` /`round.encode()`
(`simplex/scheme/mod.rs:99-105`) и что `Proposal::write` = `round ‖ parent ‖ payload`
(`simplex/types.rs:750-755`). Я НЕ прошёл до конца путь, которым
`A:evidence.rs` вырезает `msg1`/`msg2` из блоба улики, и не сверил его с
узловым фикстурным корпусом `crates/dpos/consensus/tests/equivocation_evidence_conformance.rs`
(на него ссылается `A:evidence.rs:320-325`). Мой вектор голоса в §4.2 использует
синтетическое 35-байтное сообщение, а не настоящий `proposal.encode()` — это
доказывает конвейер хеширования, но не то, что контракт вырезает ровно те байты,
которые узел подписал. Это отдельная проверка, и её стоит сделать до инлайна.

### 8.5 Не проверено: недостижимые объекты в git B

`git rev-list --all --objects` покрывает достижимые объекты по локальным и
удалённым ссылкам. Dangling-объекты и ссылки, отсутствующие локально, не
проверялись — для вывода §1 это неважно (исходник найден в другом дереве), но
формально граница есть.

### 8.6 Не проверено: держит ли `registerValidator` длину `message`

`verify` не ограничивает `message`. На пути PoP туда всегда идут ровно 96 байт
(`A:consensus.rs:145-148` пинует `compressed.len() == BLS_PUBKEY_LENGTH`), на
пути улик — `evidence.msg`. Верхней границы на `evidence.msg` я не искал; при
очень длинном сообщении `msgP` растёт и растёт число блоков SHA-256, то есть газ.
DoS-риска на первый взгляд нет (платит отправитель), но это не проверено.

### 8.7 Замечание вне рамок этой задачи

`A:contracts/staking/src/config.rs:77-81` пишет `bls_verifier` только если он
ненулевой, а `initializer.rs` нулевой адрес не отвергает — то есть контракт
можно проинициализировать без верификатора, если в нём ноль валидаторов, и
получить рабочий контракт, у которого `registerValidator` ревертит
`BlsVerifierNotConfigured` навсегда, пока губернатор не позовёт `setBlsVerifier`.
После инлайна проблема исчезает сама. Пишу одной строкой, не преследую.

---

## Прямые ответы

**Восстановим ли verify байт-точно, или порт содержит недоказуемое место.**
Восстановим полностью, недоказуемых мест нет. Исходник существует —
`solidity-contracts@f641789f:contracts/libraries/BLS12381Verifier.sol`, keccak
совпадает с метаданными артефакта, компиляция даёт те же 3823 байта кода.
Реверс байткода не понадобился ни разу. Отдельно выяснилось, что Fluent считает
EIP-2537 через **arkworks** (`contracts/bls12381` → `revm-precompile` без фичи
`blst`), а узел подписывает через **blst** — две разные реализации; их согласие
на всех векторах этого документа проверено прямым прогоном (§4.5), включая
SHA-256 (551 вход, 0 расхождений). Не доказанным осталось одно: что
rwasm-компиляция этой же реализации ведёт себя так же, как её нативная сборка.

**Три самых опасных расхождения между узлом и верификатором.**
1. Длина namespace: узел пишет полный LEB128-варинт, контракт — один байт и
   ревертит на `≥ 128`; уберут проверку при инлайне — расхождение станет
   молчаливым.
2. Порядок координат G2: на входе EIP-2537 (`x.c0` первой), на выходе
   `compressG2Unchecked` — z-cash (`x.c1` первой); перестановка половин живёт в
   одной строке `orig.sol:214` и в одной строке `encoding.rs:62-65`.
3. Проверка точки: узел проверяет подгруппу явно (`encoding.rs:45,58`),
   контракт не проверяет ничего и целиком полагается на PAIRING — так что
   `compressG1/G2Unchecked` дают настоящие коллизии (проверено: два разных
   128-байтных входа → один 48-байтный ключ), и ловит их только пара.

**Сколько мест снаружи контракта ломается молча: два.**
1. `B:devnet/local-dpos-smoke/genesis-bootstrap/src/bootstrap.rs:90-108,354` —
   своя копия `sol!` c 17 аргументами; продолжит слать `0xdfa8efb0` вместо
   нового `0xfecaf0f1`.
2. `B:devnet/local-dpos-smoke/dpos_harness/stack/production_path.py:210-211,719` —
   захардкоженная строка сигнатуры и позиционный аргумент 15.
Остальные 26 позиций (§5) ломаются громко либо это документация/артефакты; в
байткоде прочих солидити-контрактов стенда и во всём `crates/`+`bins/` ссылок нет
— проверено.

**Закрывает ли инлайн K-1 (подменный верификатор ⇒ поддельный PoP ⇒ поддельный
кворум).** Да, этот путь закрывается: `setBlsVerifier(address)` (`0x466ae541`,
губернаторский) исчезает вместе с хранимым адресом, и подменить реализацию
становится нечем. Но эквивалентный путь остаётся — он просто на этаж ниже:
`verify` целиком опирается на предеплои `0x02/0x05/0x0b/0x0f/0x10`, а на Fluent
это не встроенные предеплои, а rWasm-контракты по адресам из
`EXECUTE_USING_SYSTEM_RUNTIME_ADDRESSES` (`crates/types/src/genesis.rs:146-172`),
устанавливаемые генезисом и обновляемые через `runtime-upgrade`. Кто может
подменить код по `0x0f`, тот получает ровно ту же подделку PoP. Инлайн сужает
поверхность с «губернаторский сеттер в стейкинге» до «механизм обновления
системных предеплоев» — это заметно уже, но не ноль.

**Стоило ли делать этот шаг отдельно от порта.** Да, и не из-за реверса.
Реверс оказался не нужен вовсе — и вот это как раз то, что порт бы не выяснил:
он бы начался с реверса артефакта (как и было запланировано), потому что вопрос
«где исходник» был закрыт неверно — искали в A и B, а файл лежит в третьем
дереве, на ветке, куда чекаут сейчас не переключён. Порт, начатый с реверса,
потерял бы комментарии (в том числе security-note про кофактор), готовые
конформанс-тесты солидити в C и рукописное зеркало корпуса. Кроме того отдельно
выяснилось три вещи, которые порт бы не искал: коллизия `compress*Unchecked`
(два разных входа → один ключ), что `InvalidProofOfPossession` и
`BlsVerifierNotConfigured` не покрыты ни одним тестом, и что
`production_path.py` уже сейчас ссылается на путь `.sol`, которого нет на
текущей ветке соседнего репозитория.
