# Type-Level Programming (TLP) in Rust

## What is TLP?

TLP is, in short, _computation over types_, where
* **Types** are used as *values*
* **Generic parameters** are used as *variables*
* **Trait bounds** are used as *types*
* **Traits with associated types** are as used as *functions*

Let's see some examples of Rust TLP crates.

Example 1: TLP integers.

```rust
use typenum::{Sum, Exp, Integer, N2, P3, P4};

type X = Sum<P3, P4>;
assert_eq!(<X as Integer>::to_i32(), 7);

type Y = Exp<N2, P3>;
assert_eq!(<Y as Integer>::to_i32(), -8);
```

Example 2: TLP lists.

```rust
use type_freak::{TListType, list::*};

type List1 = TListType![u8, u16, u32];

type List2 = LPrepend<List1, u64>;
// List2 ~= TListType![u64, u8, u16, u32]
```

In this document, we will first explain how TLP works in Rust and
then will demonstrate the value of TLP by giving a good application---
implementing zero-cost capablities!

## How TLP Works?

### Case Study: TLP bools

Let's define type-level bools.

```rust
/// A marker trait for type-level bools.
pub trait Bool {} //
impl Bool for True {}
impl Bool for False {}

/// Type-level "true".
pub struct True;
/// Type-level "false".
pub struct False;
```

Let's compute over type-level bools.

```rust
/// A trait operator for logical negation on type-level bools.
pub trait Not {
	type Output;
}

impl Not for True {
    type Output = False;
}

impl Not for False {
    type Output = True;
}

/// A type alias to make using the `Not` operator easier.
pub type NotOp<B> = <B as Not>::Output;
```

Let's test the type-level bools.

```rust
use crate::bool::{True, False, NotOp};
use crate::assert_type_same;

#[test]
fn test_not_op() {
	assert_type_same!(True, NotOp<False>);
	assert_type_same!(False, NotOp<True>);
}
```

Let's define more operations for type-level bools.
```rust
/// A trait operator for logical and on type-level bools.
pub trait And<B: Bool> {
	type Output;
}

impl<B: Bool> And<B> for True {
    type Output = B;
}

impl<B: Bool> And<B> for False {
    type Output = False;
}

/// A type alias to make using the `And` operator easier.
pub type AndOp<B0, B1> = <B0 as And<B1>>::Output;
```

Let's test the and operation.
```rust
#[test]
fn test_and_op() {
	assert_type_same!(AndOp<True, True>, True);
	assert_type_same!(AndOp<True, False>, False);
	assert_type_same!(AndOp<False, True>, False);
	assert_type_same!(AndOp<False, False>, False);
}
```

### Mnemonic for TLP functions

#### Defining a function

```rust
impl SelfType {
	pub fn method(&self, arg0: Type0, arg1: Type1, /* ... *) -> RetType {
		/* function body */
	}
}
```

v.s.

```rust
impl<Arg0, Arg1, /* ... */> Method<Arg0, Arg1, /* ... */> for SelfType
where
	Arg0: ArgBound0,
	Arg1: ArgBound1,
	/* ... */
{
	type Output/*: RetBound */ = /* function body */;
}
```

Traits like `Method` are called _trait operators_.

#### Calling a function

```rust
object.method(arg0, arg1, /* ... */>);
```

v.s.

```rust
<Object as Method<Arg0, Arg1, /* ... */>>::Output
```

---

```rust
ObjectType::method(&object, arg0, arg1, /* ... */>);
```

v.s.

```rust
MethodOp<Object, Arg0, Arg1, /* ... */>
```

### TLP equality assertions

This is what we want.

```rust
#[test]
fn test_assert() {
	assert_type_same!(u16, u16);
	assert_type_same!((), ());
	// assert_same_type!(u16, bool); // Compiler error!
}
```

Here is how it is implemented.

```rust
/// A trait that is intended to check if two types are the same in trait bounds.
pub trait IsSameAs<T> {
    type Output;
}

impl<T> IsSameAs<T> for T {
    type Output = ();
}
```

```rust
pub type AssertTypeSame<Lhs, Rhs> = <Lhs as IsSameAs<Rhs>>::Output;

#[macro_export]
macro_rules! assert_type_same {
    ($lhs:ty, $rhs:ty) => {
        const _: AssertTypeSame<$lhs, $rhs> = ();
    };
}
```

### TLP equality checks

The `SameAsOp` (and `SameAs`) is a very useful primitive.

```rust
#[test]
fn test_same_as() {
	assert_type_same!(SameAsOp<True, True>, True);
	assert_type_same!(SameAsOp<False, True>, False);
	assert_type_same!(SameAsOp<True, NotOp<False>>, True);
}
```

This is how it gets implemented.

```rust
/// A trait operator to check if two types are the same, returning a Bool.
pub trait SameAs<T> {
    type Output: Bool;
}

pub type SameAsOp<T, U> = <T as SameAs<U>>::Output;
```

```rust
impl SameAs<True> for True {
    type Output = True;
}
impl SameAs<False> for True {
    type Output = False;
}

impl SameAs<True> for False {
    type Output = False;
}
impl SameAs<False> for False {
    type Output = True;
}
```

Can be simplified in the future (through the unstablized feature of specialization)

```rust
#![feature(specialization)]

impl<T> SameAs<T> for True {
    default type Output = False;
}
impl SameAs<True> for True {
    type Output = True;
}

impl<T> SameAs<T> for False {
    default type Output = False;
}
impl SameAs<False> for False {
    type Output = True;
}
```

### TLP control flow

#### Conditions

```rust
#[test]
fn test_if() {
	assert_type_same!(IfOp<True, u32, ()>, u32);
	assert_type_same!(IfOp<False, (), bool>, bool);
}
```

```rust
pub trait If<Cond: Bool, T1, T2> {
    type Output;
}

impl<T1, T2> If<True, T1, T2> for () {
    type Output = T1;
}

impl<T1, T2> If<False, T1, T2> for () {
    type Output = T2;
}

pub type IfOp<Cond, T1, T2> = <() as If<Cond, T1, T2>>::Output;
```

#### Loops

You don't write loops in TLP; you write _recursive_ functions.

### TLP collections 

Take TLP sets as an example.

```rust
use core::marker::PhantomData;

/// A marker trait for type-level sets.
pub trait Set {}

/// An non-empty type-level set.
pub struct Cons<T, S: Set>(PhantomData<(T, S)>);
/// An empty type-level set.
pub struct Nil;

impl<T, S: Set> Set for Cons<T, S> {}
impl Set for Nil {}
```

How to write a _contain_ operator?

```rust
#[test]
fn test_set_contain() {
	struct A;
	struct B;
	struct C;
	struct D;
	
	type AbcSet = Cons<A, Cons<B, Cons<C, Nil>>>;
	assert_type_same!(SetContainOp<AbcSet, A>, True);
	assert_type_same!(SetContainOp<AbcSet, B>, True);
	assert_type_same!(SetContainOp<AbcSet, C>, True);
	assert_type_same!(SetContainOp<AbcSet, D>, False);
}
```

We can implement the operator with recursion!

```rust
/// A trait operator to check if `T` is a member of a type set;
pub trait SetContain<T> {
	type Output;
}

impl<T> SetContain<T> for Nil {
	type Output = False;
}

impl<T, U, S> SetContain<T> for Cons<U, S>
where
	S: Set,
	U: SameAs<T>,
	S: SetContain<T>,
	SameAsOp<U, T>: Or<SetContainOp<S, T>>,
{
	type Output = OrOp<SameAsOp<U, T>, SetContainOp<S, T>>;
}

pub type SetContainOp<Set, Item> = <Set as SetContain<Item>>::Output;
```

Note: needs to implement `SameAs` for all possible item types (e.g., among `A` through `D`).

### Where are the boundaries for TLP?

#### Expressiveness: practically unlimited.

![[Pasted image 20210825015426.png]]

---

![[Pasted image 20210825015401.png]]

#### Ergonomics: probably fixable.

An example from the `typ` crate.

```rust
typ! {
	fn BinaryGcd<lhs, rhs>(lhs: Unsigned, rhs: Unsigned) -> Unsigned {
		if lhs == rhs {
			lhs
		} else if lhs == 0u {
			rhs
		} else if rhs == 0u {
			lhs
		} else {
			if lhs % 2u == 1u {
				if rhs % 2u == 1u {
					if lhs > rhs {
						let sub: Unsigned = lhs - rhs;
						BinaryGcd(sub, rhs)
					} else {
						let sub: Unsigned = rhs - lhs;
						BinaryGcd(sub, lhs)
					}
				} else {
					let div: Unsigned = rhs / 2u;
					BinaryGcd(lhs, div)
				}
			} else {
				if rhs % 2u == 1u {
					let div: Unsigned = lhs / 2u;
					BinaryGcd(div, rhs)
				} else {
					let ldiv: Unsigned = lhs / 2u;
					let rdiv: Unsigned = rhs / 2u;
					BinaryGcd(ldiv, rdiv) * 2u
				}
			}
		}
	}
}
```

## An Application of TLP

### Capabilities in Rust

A simplified example from zCore:

```rust
pub struct Handle<O> {
	/// The object referred to by the handle.
	pub object: Arc<O>,

	/// The handle's associated rights.
	pub rights: Rights,
}
```

The limitations
* CPU cost: check for rights cost CPU cycles;
* Memory cost: access rights cost memory space;
* Security weakness: it is easy to bypass access rights internally.

### Our idea: _zero-cost_ capabilities

Encoding the access rights in the type `R`.

```rust
pub struct Handle<O, R> {
	/// The object referred to by the handle.
    object: Arc<O>,
	
	/// The handle's associated rights.
    rights: PhantomData<R>,
}
```

*Compile-time* access control with trait bounds on `R`.

```rust
use crate::my_dummy_channel::{Channel, Msg};
use crate::rights::Read;

impl<R: Read> Handle<Channel, R> {
	pub fn read(&self) -> Msg {
		self.object.read()
	}
}

impl<R: Write> Handle<Channel, R> {
	pub fn write(&self, msg: Msg) {
		self.object.write(msg)
	}
}
```

**--> Requirement 1. Construct a set of types that _satisfies all possible combinations of the trait bounds_.**

```rust
use crate::rights::{Rights, RightsIsSuperSetOf};

impl<O, R: Rights> Handle<O, R> {
    /// Create a new handle for the given object.
    pub fn new(object: O) -> Self {
        Self {
            object: Arc::new(object),
            rights: PhantomData,
        }
    }

    /// Create a duplicate of the handle, refering to the same object.
    ///
    /// The duplicate is guaranteed---by the compiler---to have the same or
    /// lesser rights.
    pub fn duplicate<R1>(&self) -> Handle<O, R1>
    where
        R1: Rights,
        R: RightsIsSuperSetOf<R1>,
    {
        Handle {
            object: self.object.clone(),
            rights: PhantomData,
        }
    }
}
```

**--> Requirement 2. Implement the `RightsIsSuperSetOf<Sub>` trait for all `Super`,  where  _the rights represented by the `Super` type is a superset of the rights represented by the `Sub` type_.**

(Question: why a naive implementation won't work?)

### Our solution: the TLP-powered `trait_flags` crate

Our `trait_flags` crate is similar to `bitflags`, but it uses traits (or types) as flags, instead of bits.

```rust
/// Define a set of traits that represent different rights and
/// a macro that outputs types with the desired combination of rights.
trait_flags! {
    trait Rights {
        Read,
        Write,
		// many more...
    }
}
```


```rust
#[test]
fn define_rights() {
	type MyNoneRights = Rights![];
	type MyReadRights = Rights![Read];
	type MyWriteRights = Rights![Write];
	type MyReadWriteRights = Rights![Read, Write];
}
```


```rust
#[test]
fn has_rights() {
	fn has_read_right<T: Read>() {}
	fn has_write_right<T: Write>() {}

	has_read_right::<Rights![Read]>();
	has_read_right::<Rights![Read, Write]>();
	//has_read_right::<Rights![]>(); // Compiler error!

	has_write_right::<Rights![Write]>();
	has_write_right::<Rights![Read, Write]>();
	//has_write_right::<Rights![]>(); // Compiler error!
}
```

```rust
#[test]
fn rights_is_superset_of() {
	fn is_superset_of<Super: Rights + RightsIsSuperSetOf<Sub>, Sub: Rights>() {}

	is_superset_of::<Rights![Read], Rights![]>();
	is_superset_of::<Rights![Write], Rights![]>();
	is_superset_of::<Rights![Read], Rights![Read]>();
	is_superset_of::<Rights![Read, Write], Rights![Write]>();
	is_superset_of::<Rights![Read, Write], Rights![Write]>();

	//is_superset_of::<Rights![Read], Rights![Write]>(); // Compiler error!
	//is_superset_of::<Rights![], Rights![Read]>(); // Compiler error!
}
```

### How  `trait_flags` works?

Let's walkthrough the Rust code generated by the `trait_flags` macro. Without loss of generality, we assume that the input given to the macro consists of only two rights: the read and write rights.

#### For requirement 1

**Step 1.** Generate a set of the types that can represent all possible combinations of rights.

```rust
// Marker traits
pub trait Rights {}
pub trait Read: Rights {}
pub trait Write: Rights {}

pub struct RightSet<B0, B1>(
	PhantomData<B0>, // If B0 == True, then the set contains the read right
	PhantomData<B1>, // If B1 == True, then the set contains the write right
);
```

**Step 2.** .

```rust
impl<B0, B1> Rights for RightSet<B0, B1> {}
impl<B1> Read for RightSet<True, B1> {}
impl<B0> Write for RightSet<B0, True> {}
```

#### For requirement 2

**Step 1.** Reduce the problem of `RightsIsSuperSetOf` trait to implementing a trait operator named `RightsIncludeOp`.

```rust
/// A marker trait that marks any pairs of `Super: Rights` and `Sub: Rights` types,
/// where the rights of `Super` is a superset of that of `Sub`.
pub trait RightsIsSuperSetOf<R: Rights> {}

impl<Super, Sub> RightsIsSuperSetOf<Sub> for Super
where
	Super: Rights + RightsInclude<Sub>,
	Sub: Rights,
	True: IsSameAs<RightsIncludeOp<Super, Sub>>,
{
}

/// A type alias for `RightsInclude`.
pub type RightsIncludeOp<Super, Sub> = <Super as RightsInclude<Sub>>::Output;

/// A trait operator that "calculates" if `Super: Rights` is a superset of `Sub: Rights`.
/// If yes, the result is `True`; otherwise, the result is `False`.
pub trait RightsInclude<Sub: Rights> {
	type Output;
}
```

**Step 2. ** Implement `RightsIncludeOp`.

Here is a simplified version.
```rust
impl<Super0, Super1, Sub0, Sub1> RightsInclude<RightSet<Sub0, Sub1>>
	for RightSet<Super0, Super1>
where
	Super0: Bool,
	Super1: Bool,
	Sub0: Bool,
	Sub1: Bool,
	BoolGreaterOrEqual<Super0, Sub0>: And<BoolGreaterOrEqual<Super1, Sub1>>,
	// For brevity, we omit many less important trait bounds...
{
	type Output = AndOp<BoolGreaterOrEqual<Super0, Sub0>, BoolGreaterOrEqual<Super1, Sub1>>;
}

type BoolGreaterOrEqual<B0, B1> = NotOp<AndOp<SameAsOp<B0, False>, SameAsOp<B1, True>>>;
```

### An alternative implementation of `trait_flags`

Let's walkthrough the Rust code generated by this alternative implementation of `trait_flags`, which is based on variable-length TLP sets that we have described in part 1 of this talk. Again, we assume the input to the macro is only the read and write rights.

#### For requirement 1

**Step 1.** Generate a set of the types that can represent all possible combinations of rights.

```rust
use crate::set::{Set, Cons, Nil};

pub trait Rights {}
pub trait Read: Rights {}
pub trait Write: Rights {}

impl<S: Set + Rights> Rights for Cons<ReadRight, S> {}
impl<S: Set + Rights> Rights for Cons<WriteRight, S> {}
impl Rights for Nil {}

pub struct ReadRight;
pub struct WriteRight;
```

**Step 2.** Define the `SameAs` relationship between right types.

```rust
impl SameAs<ReadRight> for ReadRight {
	type Output = True;
}
impl SameAs<WriteRight> for ReadRight {
	type Output = False;
}

impl SameAs<ReadRight> for WriteRight {
	type Output = False;
}
impl SameAs<WriteRight> for WriteRight {
	type Output = True;
}
```

The above code is only part that results in a complexity greater than O(N), where N is the number of types. In the future, it can be reduced to the complexity of O(N) with the help of an unstable feature named _specialization_.

```rust
#![feature(specialization)]

impl<T> SameAs<T> for ReadRight {
	default type Output = False;
}
impl SameAs<ReadRight> for ReadRight {
	type Output = True;
}

impl<T> SameAs<T> for WriteRight {
	default type Output = False;
}
impl SameAs<WriteRight> for WriteRight {
	type Output = True;
}
```

**Step 3.** Implement the marker traits for _right_ types.

```rust
impl<R> Read for R
where
	R: Rights + SetContain<ReadRight>,
	True: IsSameAs<SetContainOp<R, ReadRight>>,
{
}

impl<R> Write for R
where
	R: Rights + SetContain<WriteRight>,
	True: IsSameAs<SetContainOp<R, WriteRight>>,
{
}
```

Note that we have implemented the `SetContain` and `SetContainOp` trait operators in part 1.

#### For requirement 2

**Step 1.** Reduce the problem of `RightsIsSuperSetOf` trait to implementing a trait operator named `RightsIncludeOp`, which in turn can be implemented with `SetIncludeOp`

```rust
/// A marker trait that marks any pairs of `Super: Rights` and `Sub: Rights` types,
/// where the rights of `Super` is a superset of that of `Sub`.
pub trait RightsIsSuperSetOf<R: Rights> {}

impl<Super, Sub> RightsIsSuperSetOf<Sub> for Super
where
	Super: Rights + RightsInclude<Sub>,
	Sub: Rights,
	True: IsSameAs<RightsIncludeOp<Super, Sub>>,
{
}

use crate::set::{SetInclude as RightsInclude, SetIncludeOp as RightsIncludeOp}
```

**Step 2.** Implement `SetIncludeOp`, which is a trait operator that checks if a set A includes a set B, i.e., the set A is a superset of the set B.

```rust
/// A trait operator to check if a set A includes a set B, i.e., A is a superset of B.
pub trait SetInclude<S> {
	type Output;
}

impl SetInclude<Nil> for Nil {
	type Output = True;
}

impl<T, S: Set> SetInclude<Cons<T, S>> for Nil {
	type Output = False;
}

impl<T, S: Set> SetInclude<Nil> for Cons<T, S> {
	type Output = True;
}

impl<SuperT, SuperS, SubT, SubS> SetInclude<Cons<SubT, SubS>> for Cons<SuperT, SuperS>
where
	SuperS: Set + SetInclude<SubS> + SetContain<SubT> + SetInclude<SubS>,
	SubS: Set,
	SuperT: SameAs<SubT>,
	// For brevity, we omit some trait bounds...
{
	type Output = IfOp<
		SameAsOp<SuperT, SubT>,     // The if condition
		SetIncludeOp<SuperS, SubS>, // The if branch
		AndOp<                      // The else branch
			SetContainOp<SuperS, SubT>,
			SetIncludeOp<SuperS, SubS>,
		>,
	>;
}

pub type SetIncludeOp<SuperSet, SubSet> = <SuperSet as SetInclude<SubSet>>::Output;
```

## Wrapup

* Part 1: the magic of TLP
	* TLP bools
	* TLP equality assertions and checks
	* TLP control flow (if and ~~loops~~)
	* TLP collections (type sets)
* Part 2: the application of TLP
	* Traditional capability-based access control 
	* Zero-cost capability-based access control 
	* How to implement `trait_flags` with TLP
		* Fixed-length set version (`RightSet<R0, R1, ...>`)
		* Variable-length set version (`Cons<R0, Cons<R1, ...>`)
