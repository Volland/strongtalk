//! Language behaviour checks: each expression's printString must match.

use serde_json::Value as J;
use strongtalk::Vm;

fn print(vm: &mut Vm, src: &str) -> String {
    let out: J = serde_json::from_str(&vm.eval(src, "test", true)).unwrap();
    if out["ok"] == true {
        out["value"].as_str().unwrap().to_string()
    } else {
        format!("ERROR: {}", out["error"].as_str().unwrap_or(""))
    }
}

fn check(cases: &[(&str, &str)]) {
    let mut vm = Vm::new();
    assert!(vm.boot_errors.is_empty(), "library problems: {:?}", vm.boot_errors);
    let mut failures = Vec::new();
    for (src, want) in cases {
        let got = print(&mut vm, src);
        if got != *want {
            failures.push(format!("{}\n    want: {}\n    got:  {}", src, want, got));
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

#[test]
fn arithmetic() {
    check(&[
        ("3 + 4 * 5", "35"),
        ("3 + (4 * 5)", "23"),
        ("7 // 2", "3"),
        ("-7 // 2", "-4"),
        ("-7 \\\\ 2", "1"),
        ("-7 rem: 2", "-1"),
        ("-7 quo: 2", "-3"),
        ("1/3 + (1/6)", "1/2"),
        ("(2/4) class name", "'Fraction'"),
        ("(4/2) class name", "'SmallInteger'"),
        ("SmallInteger maxVal", "536870911"),
        ("(SmallInteger maxVal + 1) class name", "'LargeInteger'"),
        ("(SmallInteger maxVal + 1 - 1) class name", "'SmallInteger'"),
        ("30 factorial", "265252859812191058636308480000000"),
        ("2 raisedTo: 64", "18446744073709551616"),
        ("(2 raisedTo: 64) - (2 raisedTo: 64)", "0"),
        ("1.5 + 1", "2.5"),
        ("0.1 + 0.2 > 0.3", "true"),
        ("2 sqrt * 2 sqrt", "2.0"),
        ("10 printString: 2", "'1010'"),
        ("255 printStringRadix: 16", "'16rFF'"),
        ("16r1F", "31"),
        ("2r1010", "10"),
        ("1e3", "1000"),
        ("1.5e-3", "0.0015"),
        ("17 isPrime", "true"),
        ("12 gcd: 18", "6"),
        ("3 max: 9", "9"),
        ("-5 abs", "5"),
        ("3.7 floor", "3"),
        ("3.2 ceiling", "4"),
        ("-3.5 rounded", "-4"),
        ("(1/3) asFloat", "0.333333"),
        ("100 sqrt", "10.0"),
        ("1 = 1.0", "true"),
        ("(1/2) = 0.5", "true"),
        ("3 @ 4 + (1 @ 1)", "4@5"),
        ("[1/0] on: ZeroDivide do: [:e | e return: -1]", "-1"),
        ("42 asWords", "'forty-two'"),
        ("Float pi", "3.141593"),
        ("0.1 + 0.2", "0.3"),
        ("2 sqrt", "1.414214"),
        ("1.0e-7", "0.0"),
        ("(SmallInteger maxVal + 1) printString", "'536870912'"),
        ("SmallInteger minVal", "-536870912"),
        ("{{primitiveMandelbrotAtRe: 0.0 im: 0.0 iterate: 100 ifFail: [:e | e]}}", "99"),
        ("{{primitiveMandelbrotAtRe: 2.0 im: 2.0 iterate: 100 ifFail: [:e | e]}}", "0"),
        ("{{primitiveMandelbrotAtRe: 1 im: 0.0 iterate: 100 ifFail: [:e | e]}}", "#FirstArgumentHasWrongType"),
        ("{{primitiveMandelbrotAtRe: 0.0 im: 0.0 iterate: 1.5 ifFail: [:e | e]}}", "#ThirdArgumentHasWrongType"),
        ("{{#(1 2) primitiveIndexedObjectAt: 3 ifFail: [:e | e]}}", "#OutOfBounds"),
        ("{{#(1 2) primitiveIndexedObjectAt: $a ifFail: [:e | e]}}", "#FirstArgumentHasWrongType"),
        ("{{1 primitiveDiv: 0 ifFail: [:e | e]}}", "#DivisionByZero"),
        ("{{1 primitiveBitShift: 28 ifFail: [:e | e]}}", "268435456"),
        ("{{1 primitiveBitShift: 29 ifFail: [:e | e]}}", "#SmallIntegerOverflow"),
        ("{{3 primitiveBitShift: 28 ifFail: [:e | e]}}", "#SmallIntegerOverflow"),
        ("{{-7 primitiveBitShift: -1 ifFail: [:e | e]}}", "-4"),
        ("{{-1 primitiveBitShift: -100 ifFail: [:e | e]}}", "-1"),
        ("1 bitShift: 100", "1267650600228229401496703205376"),
        ("(1 bitShift: 100) bitShift: -99", "2"),
        ("-7 >> 1", "-4"),
        ("(Character value: 255) value", "255"),
        ("{{primitiveCharacterFor: 256 ifFail: [:e | e]}}", "#OutOfBounds"),
        ("{{primitiveCharacterFor: -1 ifFail: [:e | e]}}", "#OutOfBounds"),
        ("{{primitiveCharacterFor: $a ifFail: [:e | e]}}", "#FirstArgumentHasWrongType"),
        ("2.5 rounded", "3"),
        ("| s | s := 'abc' copy. s at: 2 put: $X. s", "'aXc'"),
        ("| s | s := 'abc' copy. {{s primitiveIndexedByteAt: 1 put: 65 ifFail: [:e | e]}}", "'Abc'"),
        ("{{('abc' copy) primitiveIndexedByteAt: 1 put: 256 ifFail: [:e | e]}}", "#ValueOutOfRange"),
        ("{{('abc' copy) primitiveIndexedByteAt: 1 put: $a ifFail: [:e | e]}}", "#SecondArgumentHasWrongType"),
        ("{{('abc' copy) primitiveIndexedByteAt: 9 put: 65 ifFail: [:e | e]}}", "#OutOfBounds"),
        ("[('abc' copy) at: 5 put: $z] on: SubscriptOutOfBounds do: [:e | #caught]", "#caught"),
        ("-3.7 truncated", "-3"),
        ("3.7 truncated", "3"),
        ("-3.2 floor", "-4"),
        ("1.0e15 truncated", "1000000000000000"),
        ("{{-3.7 primitiveFloatTruncated}}", "-3.0"),
        ("{{3.5 primitiveFloatAsSmallIntegerIfFail: [:e | e]}}", "#SmallIntegerConversionFailed"),
        ("{{3.0 primitiveFloatAsSmallIntegerIfFail: [:e | e]}}", "3"),
        ("{{1.0e10 primitiveFloatSmallIntegerFloorIfFail: [:e | e]}}", "#ConversionFailed"),
        ("-1 sqrt", "-1.#IND"),
        ("Float infinity", "1.#INF"),
        ("-2.5 rounded", "-3"),
        ("{{536870911.0 primitiveFloatRoundedAsSmallIntegerIfFail: [:e | e]}}", "#SmallIntegerConversionFailed"),
        ("1.0e12 rounded", "1000000000000"),
        ("'abc' < 'abd'", "true"),
        ("{{'abc' primitiveIndexedByteCompare: 'ab' ifFail: [:e | e]}}", "1"),
        ("[nil foo: 3] on: MessageNotUnderstood do: [:e | e message receiver printString, ' ', e message arguments printString]", "'nil #(3)'"),
        ("{{Array primitiveNew: -1 ifFail: [:e | e]}}", "#NegativeSize"),
    ]);
}

#[test]
fn collections_and_strings() {
    check(&[
        ("#(3 1 2) asSortedCollection asArray", "#(1 2 3)"),
        ("(1 to: 5) collect: [:i | i * i]", "#(1 4 9 16 25)"),
        ("(1 to: 10) select: [:i | i even]", "#(2 4 6 8 10)"),
        ("(1 to: 10) inject: 0 into: [:a :b | a + b]", "55"),
        ("#(1 2 3) reverse", "#(3 2 1)"),
        ("#(1 $a 'b' #c nil) printString", "'#(1 $a ''b'' #c nil)'"),
        ("(OrderedCollection new add: 1; add: 2; addFirst: 0; yourself) asArray", "#(0 1 2)"),
        ("| oc | oc := OrderedCollection new. 1 to: 100 do: [:i | oc add: i]. oc removeFirst; removeLast. oc size", "98"),
        ("(Set withAll: #(1 2 2 3 3 3)) size", "3"),
        ("| d | d := Dictionary new. d at: #a put: 1; at: #b put: 2. (d at: #b) + (d at: #a)", "3"),
        ("| d | d := Dictionary new. d at: 'x' put: 1. d at: 'x' ifAbsent: [0]", "1"),
        ("| d | d := Dictionary new. d at: #zz ifAbsent: ['none']", "'none'"),
        ("| d | d := Dictionary new. 1 to: 1000 do: [:i | d at: i put: i * 2]. (d at: 777)", "1554"),
        ("(Bag withAll: #(a b a c a)) occurrencesOf: #a", "3"),
        ("#(5 2 8 1) asSortedCollection: [:a :b | a > b]", "a SortedCollection(8 5 2 1)"),
        ("'hello' reversed", "'olleh'"),
        ("'hello' asUppercase", "'HELLO'"),
        ("'hello world' substrings", "#('hello' 'world')"),
        ("'hello' , ' ' , 'world'", "'hello world'"),
        ("'abc' < 'abd'", "true"),
        ("'abc' = 'abc'", "true"),
        ("#abc == 'abc' asSymbol", "true"),
        ("'hello' indexOf: $l", "3"),
        ("'hello' includesSubstring: 'ell'", "true"),
        ("'a,b,c' substrings: ','", "#('a' 'b' 'c')"),
        ("'42' asNumber + 1", "43"),
        ("'it''s' size", "4"),
        ("$a asUppercase", "$A"),
        ("$a value", "97"),
        ("(String new: 3) size", "3"),
        ("| s | s := WriteStream on: String new. s nextPutAll: 'x = '; print: 3; nextPut: $!. s contents", "'x = 3!'"),
        ("(ReadStream on: #(1 2 3)) next; next", "2"),
        ("('one two' readStream upTo: $ )", "'one'"),
        ("#(1 2 3) , #(4)", "#(1 2 3 4)"),
        ("#(3 1 2) asArray sort", "#(1 2 3)"),
        ("(1 to: 0) asArray", "#()"),
        ("#(1 2 3) detect: [:x | x > 5] ifNone: ['none']", "'none'"),
        ("#(1 2 3) allSatisfy: [:x | x > 0]", "true"),
        ("#[1 2 255]", "#[1 2 255]"),
        ("', ' join: #('a' 'b' 'c')", "'a, b, c'"),
    ]);
}

#[test]
fn blocks_classes_and_exceptions() {
    check(&[
        ("[:x :y | x + y] value: 1 value: 2", "3"),
        ("[:x | x] numArgs", "1"),
        ("[] value", "nil"),
        ("| n | n := 0. [n < 10] whileTrue: [n := n + 1]. n", "10"),
        ("| n | n := 0. 5 timesRepeat: [n := n + 2]. n", "10"),
        ("| n | n := 0. 1 to: 10 by: 2 do: [:i | n := n + i]. n", "25"),
        ("| b | b := [:x | x > 2 ifTrue: ['big'] ifFalse: ['small']]. (b value: 1), (b value: 5)", "'smallbig'"),
        ("#(1 2 3) do: [:x | x = 2 ifTrue: [^x * 100]]. 0", "200"),
        ("nil foo", "ERROR: nil does not understand #foo"),
        ("3 zork: 4", "ERROR: a SmallInteger does not understand #zork:"),
        ("[nil foo] on: MessageNotUnderstood do: [:e | e message selector]", "#foo"),
        ("[Error signal: 'boom'] on: Error do: [:e | e messageText]", "'boom'"),
        ("[Error new signal: 'x'. 5] on: Error do: [:e | 7]", "7"),
        ("[(Warning signal: 'w') ifNil: [1] ifNotNil: [:v | v]] on: Warning do: [:e | e resume: 9]", "9"),
        ("| n | n := 0. [n := n + 1. n < 3 ifTrue: [Error signal: 'again']. n] on: Error do: [:e | e retry]", "3"),
        ("| log | log := OrderedCollection new. [[log add: 1. Error signal: 'x'] ensure: [log add: 2]] on: Error do: [:e | log add: 3]. log asArray", "#(1 3 2)"),
        ("[[1/0] on: ZeroDivide do: [:e | e pass]] on: ZeroDivide do: [:e | e return: #outer]", "#outer"),
        ("[#(1 2) at: 5] on: SubscriptOutOfBounds do: [:e | #caught]", "#caught"),
        ("[nil foo] on: ZeroDivide, MessageNotUnderstood do: [:e | #set]", "#set"),
        ("Notification signal", "nil"),
        ("3 class", "SmallInteger"),
        ("3 class class", "SmallInteger class"),
        ("3 class class class", "Metaclass"),
        ("OrderedCollection superclass", "SequenceableCollection"),
        ("3 isKindOf: Integer", "true"),
        ("3 respondsTo: #printString", "true"),
        ("3 respondsTo: #zork", "false"),
        ("3 perform: #+ with: 4", "7"),
        ("#(1 2) perform: #at: with: 2", "2"),
        ("Object subclass: #Animal instanceVariableNames: 'name' classVariableNames: 'Count' category: 'zoo'. Animal compile: 'name ^name'. Animal compile: 'name: n name := n'. (Animal new name: 'Rex'; yourself) name", "'Rex'"),
        ("Animal subclass: #Dog instanceVariableNames: '' classVariableNames: '' category: 'zoo'. Dog compile: 'speak ^name , '' says woof'''. (Dog new name: 'Fido'; yourself) speak", "'Fido says woof'"),
        ("Animal class compile: 'count ^Count ifNil: [0]'. Animal class compile: 'bump Count := self count + 1'. Animal bump; bump. Dog count", "2"),
        ("Dog compile: 'name ^''Dog '' , super name'. (Dog new name: 'Rex'; yourself) name", "'Dog Rex'"),
        ("Delta define: #Pair as: ((Class subclassOf: 'Object' instanceVariables: 'a <Int> b <Int>')). Pair compile: 'a: x <Int> b: y <Int> ^<Self> a := x. b := y'. Pair compile: 'sum ^<Int> ^a + b'. (Pair new a: 3 b: 4) sum", "7"),
        ("(Pair new a: 1 b: 2) printString", "'a Pair'"),
        ("Pair new a", "ERROR: a Pair does not understand #a"),
        ("| x | x := guaranteed <Int> 5. x", "5"),
        ("Compiler evaluate: '6 * 7'", "42"),
        ("Smalltalk at: #Answer put: 42. Answer", "42"),
        ("(Smalltalk includesKey: #Answer)", "true"),
        ("ThisIsUndefined", "ERROR: undefined variable 'ThisIsUndefined'"),
        ("workspaceVar", "nil"),
    ]);
}

#[test]
fn recursion_limits() {
    let mut vm = Vm::new();
    vm.eval("Object subclass: #Deep instanceVariableNames: '' classVariableNames: '' category: 'x'. Deep compile: 'down: n ^n = 0 ifTrue: [0] ifFalse: [1 + (self down: n - 1)]'. Deep compile: 'forever ^self forever'", "", false);
    assert_eq!(print(&mut vm, "Deep new down: 2000"), "2000");
    let r = print(&mut vm, "Deep new forever");
    assert!(r.starts_with("ERROR: Stack overflow"), "{}", r);
    // the VM is still usable afterwards
    assert_eq!(print(&mut vm, "3 + 4"), "7");
}

#[test]
fn file_out_and_back_in() {
    let mut vm = Vm::new();
    let src = vm.file_out("BankAccount");
    assert!(src.contains("Delta define: #BankAccount"));
    let mut fresh = Vm::new();
    fresh.eval("Smalltalk removeKey: #BankAccount", "", false);
    let rep: J = serde_json::from_str(&fresh.file_in(&src)).unwrap();
    assert_eq!(rep["errors"].as_array().unwrap().len(), 0, "{}", rep);
    assert_eq!(print(&mut fresh, "((BankAccount owner: 'Ada') deposit: 10; yourself) balance"), "10");
}

#[test]
fn type_checker() {
    let mut vm = Vm::new();
    let all: J = serde_json::from_str(&vm.typecheck_all()).unwrap();
    assert_eq!(all["problems"].as_array().unwrap().len(), 0, "library should type-check: {}", all["problems"]);
    let bad: J = serde_json::from_str(&vm.typecheck_doit("| n <Int> | n := 'x'. n foo")).unwrap();
    let msgs: Vec<String> = bad["problems"].as_array().unwrap().iter().map(|p| p["message"].as_str().unwrap().to_string()).collect();
    assert!(msgs.iter().any(|m| m.contains("assigning String")), "{:?}", msgs);
    assert!(msgs.iter().any(|m| m.contains("does not understand #foo")), "{:?}", msgs);
    let arg: J = serde_json::from_str(&vm.typecheck_doit("(BankAccount owner: 'Ada') deposit: 'lots'")).unwrap();
    assert_eq!(arg["problems"].as_array().unwrap().len(), 1, "{}", arg);
    let ok: J = serde_json::from_str(&vm.typecheck_doit("| x | x := 'hello'. x squared")).unwrap();
    assert_eq!(ok["problems"].as_array().unwrap().len(), 0);
    let unk: J = serde_json::from_str(&vm.typecheck_doit("| x <Zork> | x")).unwrap();
    assert_eq!(unk["problems"].as_array().unwrap().len(), 1);
}
