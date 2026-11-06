# Mechanics recovered from strongtalk-r200.exe

What the original Strongtalk VM (32-bit Windows build, MSVC, `c:\cygwin\home\steve\strongtalk\vm\...`)
actually does, read from its machine code, and where the Rust rebuild uses it. Addresses are
virtual addresses in the original program. The image and class library (`strongtalk.bst`) are not in
the exe, so everything here is VM-level behaviour.

## Primitive table

Primitives are described by a table in `.data` (about 0x56e000–0x572000). Each entry holds the
primitive's name followed by a pointer to its C++ code and a flags word:

| Primitive | Name string | Table entry | Code |
|---|---|---|---|
| `primitiveMandelbrotAtRe:im:iterate:ifFail:` | 0x532870 | 0x5706fc | 0x4ccbd0 |
| `primitiveFloatPrintString` | 0x533754 | 0x56fa24 | 0x4cc840 |
| `primitiveFloatPrintFormat:ifFail:` | 0x533770 | 0x56fa08 | 0x4cc6c0 |
| `primitiveIndexedObjectAt:ifFail:` | 0x532c94 | 0x57038c | 0x4d43e0 |
| `primitiveIndexedObjectAt:put:ifFail:` | 0x532c6c | 0x5703b8 | 0x4d4520 |
| `primitiveAdd:ifFail:` | 0x52f970 | 0x56e8bc | 0x4c2800 |
| `primitiveMultiply:ifFail:` | 0x52f938 | 0x570cb8 | 0x4c2860 |
| `primitiveDiv:` / `Mod:` / `Quo:` / `Remainder:` | 0x533c18 / 0x5321f4 / 0x5316a0 / 0x531630 | | 0x4c28c0 / 0x4c2890 / 0x4c28f0 / 0x4c2920 |

The SmallInteger arithmetic entries (0x4c2800–0x4c2943) are stubs that report
`..\..\vm\prims\smi_prims.cpp` line 0xa3 "should not reach here" (0x4036d0): the real
SmallInteger code is machine code the VM generates at start-up, so it is not in the file. Their
behaviour is therefore not recovered beyond the error codes below.

## Object representation (used: `SMI_MAX`/`SMI_MIN` in `vm/src/object.rs`)

Every primitive tests oops with `and al, 3`:
* tag `00` = SmallInteger (`test bl, 3; je` at 0x4cccb9, 0x4d4484), value decoded with `sar 2`
  (0x4cccd6, 0x4d44c2) and encoded with `lea eax, [ecx*4]` (0x4ccd35);
* tag `01` = heap object, class pointer at offset +3 (`cmp [esi+3], doubleKlass` at 0x4ccc67).

So a SmallInteger is a 30-bit signed value: **`SmallInteger maxVal` = 536870911 (2^29 − 1),
`minVal` = −536870912**. The first build used 31 bits; it now uses 30, so `SmallInteger maxVal + 1`
is a LargeInteger at the same point as in the original.

## Primitive failure codes (used: `vm/src/prims.rs`, `vm/lib/strings.dlt`)

`sub_40f100` interns the VM's symbols into slots starting at 0x576490, in this order (slot =
0x576490 + 4 × index). The first entries are the failure codes primitives answer to their
`ifFail:` blocks:

| index | slot | symbol |
|---|---|---|
| 0 | 0x576490 | `SmallIntegerOverflow` |
| 1 | 0x576494 | `DivisionByZero` |
| 2 | 0x576498 | `ReceiverHasWrongType` |
| 3 | 0x57649c | `DivisionNotExact` |
| 4–13 | 0x5764a0… | `FirstArgumentHasWrongType` … `NinthArgumentHasWrongType` |
| 14–17 | | `ArgumentHasWrongType`, `ArgumentIsInvalid`, `ConversionFailed`, `NotFound` |
| 49–51 | 0x576554… | `OutOfBounds`, `IndexNotValid`, `NotYetImplemented` |
| later | | `ValueOutOfRange`, `PrimitiveNotDefined`, `SmallIntegerConversionFailed`, `NotIndexable`, `NegativeSize`, `FailedAllocation`, `StackOverflow`, `NLRError`, `FloatError`, … (136 in all) |

The rebuild's primitives now fail with these names instead of the invented ones
(`WrongArgumentType` → `FirstArgumentHasWrongType`/`Second…`/`Third…`, `IndexOutOfBounds` →
`OutOfBounds`, `KeyNotFound` → `NotFound`, `ClassNotIndexable` → `NotIndexable`, negative sizes →
`NegativeSize`, too-large sizes → `FailedAllocation`, bad shifts → `ValueOutOfRange`).

## Indexing: `primitiveIndexedObjectAt:ifFail:` (0x4d43e0)

1. If the index is not a SmallInteger (tag ≠ 00, 0x4d4484) → fail with slot 0x5764a0 =
   **`FirstArgumentHasWrongType`**.
2. Index decoded with `sar 2` and range-checked by 0x4bd550 (1-based); out of range → slot
   0x576554 = **`OutOfBounds`**.
3. Otherwise answer the element (0x40e5c0).

Used as is in `prims.rs` (`index_arg`).

## Mandelbrot: `primitiveMandelbrotAtRe:im:iterate:ifFail:` (0x4ccbd0)

* `re` must be a Float (class word compared with the Double class at 0x56d178) or the primitive
  fails with **`FirstArgumentHasWrongType`** (0x5764a0); `im` likewise with
  **`SecondArgumentHasWrongType`** (0x5764a4); the count must be a SmallInteger or
  **`ThirdArgumentHasWrongType`** (0x5764a8).
* Loop (x87 code 0x4cccd3–0x4ccd2b):
  ```
  limit := n − 1                       (sar 2; dec ebx)
  x := re. y := im. x2 := re*re. y2 := im*im. i := 0
  while i < limit:
      if x2 + y2 > 4.0: stop           (fcomp with the f64 constant 4.0 at 0x54abf8)
      i := i + 1
      y := 2*x*y + im
      x := x2 − y2 + re
      x2 := x*x. y2 := y*y
  answer i as a SmallInteger
  ```
  So z starts at c (not 0), at most n − 1 steps are taken, and the escape test is strictly
  greater than 4.0. The first build started at z = 0 and allowed n steps and Integer arguments.
  `prims.rs::mandel` now follows this exactly, `Mandelbrot iterationsAtRe:im:limit:` (the pure
  Smalltalk version in `demos.dlt`) takes the same steps, and the colouring treats `n − 1` as
  "inside the set".

## Bit shifts: `primitiveBitShift:ifFail:` (0x4c3170, table entry 0x56efa4)

Unlike `+ - *`, this SmallInteger primitive is ordinary C++ code:

* argument not a SmallInteger → **`FirstArgumentHasWrongType`** (slot 0x5764a0, 0x4c3204);
* `n > 0` (left shift, 0x4c3244–0x4c3275): if `n ≥ 29` (`cmp edx, 0x1d`), or if adding
  `1 << (31 − n)` to the tagged receiver leaves any bit set under the mask `−1 << (32 − n)`, the
  result would not fit in 30 bits and the primitive fails with **`SmallIntegerOverflow`**
  (slot 0x576490, 0x4c3278); otherwise it answers `receiver << n`;
* `n ≤ 0` (right shift, 0x4c3282–0x4c3294): `n` is clamped to `−29` (`cmp edx, −0x1d`), then an
  arithmetic shift right by `−n` (rounds toward minus infinity) with the tag bits cleared
  (`and eax, −4`).

Used in `prims.rs::bits` for SmallInteger receivers; `Integer>>bitShift:` in `numbers.dlt` now
catches `#SmallIntegerOverflow` and redoes the shift with LargeIntegers (`self * (2 raisedTo:
n)`), the way the original leaves overflow to Smalltalk code. Before, the rebuild shifted with
LargeIntegers directly and never failed.

## Message not understood: `Delta::does_not_understand` (0x43a170, `..\..\vm\runtime\delta.cpp`)

1. Creates an instance of the global named `Message` (string 0x52c6e4, `sub_40ec40`).
2. Copies the arguments into a new Array (`sub_419640`, filled by `sub_4198f0` in a loop).
3. Stores into the Message, in this order: **receiver** at +7 (0x43a26c), **selector** at +11
   (0x43a2c7), **arguments** at +15 (0x43a31d). So `Message` has three instance variables:
   `receiver selector arguments`.
4. Looks up `doesNotUnderstand:` (0x52c690) in the receiver's class (the SmallInteger class at
   0x56d170 for tag-00 receivers) with `sub_43c330`; if even that fails it prints
   `LOOKUP ERROR` … ` not found` (0x52c680, 0x52c674) and stops the VM.
5. Otherwise sends `doesNotUnderstand:` with the Message.

The rebuild's `Message` had only `selector arguments`; it now has `receiver selector
arguments` (`kernel.dlt`, `interp.rs`) and `vm.rs::dispatch_dnu` fills them in that order, so
`e message receiver` works in handlers.

## Characters: `primitiveCharacterFor:ifFail:` (0x4bd810, table entry 0x56f160)

* argument not a SmallInteger → **`FirstArgumentHasWrongType`** (0x4bd85f, slot 0x5764a0);
* value decoded with `sar 2` and compared **unsigned** with 256 (`cmp eax, 0x100; jae`,
  0x4bd870), so negative codes fail too → **`OutOfBounds`** (slot 0x576554);
* otherwise answers element `value + 1` of the VM's 256-entry character table (array at
  0x56d1d4, `inc eax` then `objArray at:` 0x40e5c0).

So the original has exactly 256 Characters. The rebuild's `Character value:` accepted any
Unicode code; it now follows this rule (`prims.rs`). Characters typed into source text outside that
range still work in strings.

## Storing into strings: `primitiveIndexedByteAt:put:ifFail:` (0x4d3210, entry 0x56fdc0)

The value stored is a **byte (a SmallInteger)**, not a Character, and the primitive answers the
**receiver**. Checks, in order (0x4d32cf–0x4d3368):

1. index not a SmallInteger → **`FirstArgumentHasWrongType`** (slot 0x5764a0);
2. value not a SmallInteger → **`SecondArgumentHasWrongType`** (slot 0x5764a4);
3. index outside `1..size` (0x4d2c80) → **`OutOfBounds`** (slot 0x576554);
4. value ≥ 256, compared unsigned so negatives too → **`ValueOutOfRange`** (slot 0x57656c);
5. the low byte is stored (`mov [eax], bl`).

`primitiveIndexedByteCharacterAt:ifFail:` (0x4d3380) checks its index the same way
(`FirstArgumentHasWrongType`, then `OutOfBounds` via 0x4d2c80).

The rebuild's version took a Character and answered it; it now follows the rule above
(`prims.rs::byte_at_put`), and `String>>at:put:` (`strings.dlt`) passes `c value` and answers
`c`, like a Smalltalk method wrapped around this primitive would.

## String comparison: `primitiveIndexedByteCompare:ifFail:` (0x4cd3a0)

* identical receiver and argument → `0` straight away (0x4cd447);
* a byte-array argument → 0x429a90; a double-byte argument → 0x429b30; anything else →
  **`FirstArgumentHasWrongType`**; the result is converted to a SmallInteger (`add eax, eax` twice);
* 0x429a90 compares the common length a 4-byte word at a time; at the first unequal word,
  0x4294e0 compares its bytes **as unsigned values** (`movzx`) and answers −1 or 1; when the
  common part is equal the shorter string is smaller: −1 / 0 / 1 by length (0x429af2–0x429b1b).

That is plain lexicographic order on unsigned bytes, then length, which is what the rebuild's
`Vec<char>` comparison already does: no change needed, now documented.

## Rounding: `primitiveFloatRoundedAsSmallIntegerIfFail:` (0x4cc480, table entry 0x56fa48)

* receiver must be a Float (class word = Double class 0x56d178);
* `x ≥ 0`: `floor(x + 0.5)` (adds the f64 0.5 at 0x54ab78, calls 0x4ec7c0);
  `x < 0`: `ceil(x − 0.5)` (0x4ec8f0) — half-way cases round away from zero;
* the result must be strictly greater than −536870912.0 (0x54ab28) and strictly less than
  536870911.0 (0x54ab20); otherwise **`SmallIntegerConversionFailed`** (slot 0x576578).

The rebuild now has this primitive with this exact rule, and `Float>>rounded` (`numbers.dlt`)
uses it, falling back to LargeInteger rounding when it fails. Before, `rounded` used Rust's
`f64::round` and went to LargeIntegers directly.

## Float to integer: truncation, floor, conversion

All three check that the receiver is a Float (class word = Double class 0x56d178) and use the
same window as rounding: a result must be strictly between −536870912.0 (0x54ab28) and
536870911.0 (0x54ab20).

| Primitive | Code | Rule | Fails with |
|---|---|---|---|
| `primitiveFloatTruncated` | 0x4cc2b0 (entry 0x56fb64) | answers a **Float**: `floor(x)` if `x > 0`, else `ceil(x)` (calls 0x4ec7c0 / 0x4ec8f0, boxes with 0x4cad00) | never fails |
| `primitiveFloatSmallIntegerFloorIfFail:` | 0x4cbfb0 (entry 0x56fa88) | `floor(x)`, converted with `ftol` (0x4eb3c0) | **`ConversionFailed`** (slot 0x5764d0) |
| `primitiveFloatAsSmallIntegerIfFail:` | 0x4cc5a0 (entry 0x56f71c) | only if `floor(x) == x` (`fucom` at 0x4cc63e): the Float already is a whole number | **`SmallIntegerConversionFailed`** (slot 0x576578) |
| `primitiveFloatSqrtIfFail:` | 0x4cba90 (entry 0x56faac) | C library `sqrt` (0x4ebf70), result boxed; a negative receiver gives NaN | never fails for Floats |

The rebuild now has these primitives with these rules. `Float>>truncated` does what they imply:
cut toward zero as a Float, then convert to a SmallInteger. `Float>>floor` uses the
SmallInteger floor primitive. Both fall back to LargeIntegers when the primitive fails
(`numbers.dlt`).

## Float printing: `primitiveFloatPrintString` (0x4cc840)

`sprintf(buffer, "%1.6f", value)` (format string at 0x54abb4, call at 0x4cc97e), then, at
0x4cc988–0x4cc9a0, while the last character is `'0'` and the one before it is not `'.'`, the
last character is dropped. One digit therefore always follows the point:

| value | original prints |
|---|---|
| `Float pi` | `3.141593` |
| `0.1 + 0.2` | `0.3` |
| `2 sqrt` | `1.414214` |
| `100 sqrt` | `10.0` |
| `1.0e-7` | `0.0` |
| `1.0e100` | all integer digits, then `.0` |

The `sprintf` called at 0x4cc97e is Microsoft's C library, linked into the exe (MSVC
toolchain, Rich header). That library writes NaN as `-1.#IND00` (the x87 "indefinite" NaN that
`0.0 / 0.0` and `sqrt(−1)` produce) and infinity as `1.#INF00` / `-1.#INF00`; the zero-trimming
above turns these into **`-1.#IND`**, **`1.#INF`** and **`-1.#INF`**, which the rebuild now prints
(derived from the library's documented behaviour, not from executing the exe).

The first build printed the shortest round-trip form (`0.30000000000000004`, `1.0e100`); it now
uses this rule (`prims.rs::format_float`).

## Inlined control messages (used: `vm/src/compiler.rs`)

The same symbol list (sub_40f100) gives the compiler's special selectors: `ifTrue:`, `ifFalse:`,
`ifTrue:ifFalse:`, `ifFalse:ifTrue:`, `and:`, `or:`, `whileTrue`, `whileTrue:`, `whileFalse`,
`whileFalse:`, `to:by:do:`, `value`, `value:`. The rebuild inlines exactly these (with literal
block arguments; `to:do:` as `to:by:do:` with step 1) and fast-paths `value`/`value:` sends to
blocks. `ifNil:`, `ifNotNil:`, `timesRepeat:` and `repeat`, which the first build inlined, are
now ordinary message sends answered by the class library, as in the original.

## Other facts

* Image and settings files: `strongtalk.bst` (0x525f78) and `.strongtalkrc` (0x525f68); log
  `strongtalk.log` (0x524450).
* Banner: "Smalltalk Virtual Machine by LongView Technologies LLC … (c) 1994-1997 Sun
  Microsystems" (0x52d6a8).
* `doesNotUnderstand:` is the selector the VM sends on lookup failure (0x52c690, used by
  `..\..\vm\runtime\delta.cpp` code at 0x43a170).

## Not recovered

* SmallInteger `+ - * // \\` overflow and rounding rules: generated at run time (see above).
  The `SmallIntegerOverflow` code shows that the original primitive fails on overflow and lets
  Smalltalk code retry with LargeIntegers; the rebuild's primitive switches to LargeInteger
  directly, which gives the same results.
* The class library, the Launcher, browsers and workspaces: they live in `strongtalk.bst`, which
  is not part of the exe; the rebuild's library and windows are new work.
