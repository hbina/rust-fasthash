use core::cell::RefCell;
use core::hash::{BuildHasher, Hasher};
use core::marker::PhantomData;
use std::io;

use derive_more::{Deref, DerefMut};
use num_traits::PrimInt;
use rand::Rng;
use xoroshiro128::Xoroshiro128Rng;

/// Generate a good, portable, forever-fixed hash value
pub trait Fingerprint<T: PrimInt> {
    /// This is intended to be a good fingerprinting primitive.
    fn fingerprint(&self) -> T;
}

#[doc(hidden)]
pub trait BuildHasherExt: BuildHasher {
    type FastHasher: FastHasher;
}

/// Fast non-cryptographic hash functions
pub trait FastHash: BuildHasherExt {
    /// The output hash generated value.
    type Hash: PrimInt;
    /// The seed to generate hash value.
    type Seed: Default + Copy;

    /// Hash functions for a byte array.
    /// For convenience, a seed is also hashed into the result.
    fn hash_with_seed<T: AsRef<[u8]>>(bytes: T, seed: Self::Seed) -> Self::Hash;

    /// Hash functions for a byte array.
    fn hash<T: AsRef<[u8]>>(bytes: T) -> Self::Hash {
        Self::hash_with_seed(bytes, Default::default())
    }
}

/// Fast non-cryptographic hasher
pub trait FastHasher: Hasher
where
    Self: Sized,
{
    /// The seed to generate hash value.
    type Seed: Default + Copy + From<Seed>;

    /// The output type
    type Output;

    /// Constructs a new `FastHasher`.
    #[inline(always)]
    fn new() -> Self {
        Self::with_seed(Default::default())
    }

    /// Constructs a new `FastHasher` with a random seed.
    fn with_random_seed() -> Self {
        Self::with_seed(Seed::gen().into())
    }

    /// Constructs a new `FastHasher` with seed.
    fn with_seed(seed: Self::Seed) -> Self;
}

/// Hasher in the buffer mode for short key
pub trait BufHasher: FastHasher + AsRef<[u8]> {
    /// Constructs a buffered hasher with capacity and seed
    fn with_capacity_and_seed(capacity: usize, seed: Option<Self::Seed>) -> Self;

    /// Returns the number of bytes in the buffer.
    #[inline(always)]
    fn len(&self) -> usize {
        self.as_ref().len()
    }

    /// Returns `true` if the slice has a length of 0.
    #[inline(always)]
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Extracts a slice containing the entire buffer.
    #[inline(always)]
    fn as_slice(&self) -> &[u8] {
        self.as_ref()
    }
}

/// Hasher in the streaming mode without buffer
pub trait StreamHasher: FastHasher + Sized {
    /// Writes the stream into this hasher.
    fn write_stream<R: io::Read>(&mut self, r: &mut R) -> io::Result<usize> {
        let mut buf = [0_u8; 4096];
        let mut len = 0;
        let mut pos = 0;
        let ret;

        loop {
            if pos == buf.len() {
                self.write(&buf[..]);
                pos = 0;
            }

            match r.read(&mut buf[pos..]) {
                Ok(0) => {
                    ret = Ok(len);
                    break;
                }
                Ok(n) => {
                    len += n;
                    pos += n;
                }
                Err(ref e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => {
                    ret = Err(e);
                    break;
                }
            }
        }

        if pos > 0 {
            self.write(&buf[..pos])
        }

        ret
    }
}

/// A trait which represents the ability to hash an arbitrary stream of bytes.
pub trait HasherExt: Hasher {
    /// Completes a round of hashing, producing the output hash generated.
    fn finish_ext(&self) -> u128;
}

/// Generate hash seeds
///
/// It base on the same workflow from `std::collections::RandomState`
///
/// > Historically this function did not cache keys from the OS and instead
/// > simply always called `rand::thread_rng().gen()` twice. In #31356 it
/// > was discovered, however, that because we re-seed the thread-local RNG
/// > from the OS periodically that this can cause excessive slowdown when
/// > many hash maps are created on a thread. To solve this performance
/// > trap we cache the first set of randomly generated keys per-thread.
///
/// > Later in #36481 it was discovered that exposing a deterministic
/// > iteration order allows a form of DOS attack. To counter that we
/// > increment one of the seeds on every `RandomState` creation, giving
/// > every corresponding `HashMap` a different iteration order.
///
/// # Examples
///
/// ```
/// use fasthash::{Seed, city};
///
/// city::hash128_with_seed(b"hello world", Seed::gen().into());
/// ```
#[derive(Clone, Copy, Debug, Deref, DerefMut)]
pub struct Seed(Xoroshiro128Rng);

impl Seed {
    #[inline(always)]
    fn new() -> Seed {
        Seed(Xoroshiro128Rng::new())
    }

    /// Generate a new seed
    #[inline(always)]
    pub fn gen() -> Seed {
        thread_local!(static SEEDS: RefCell<Seed> = RefCell::new(Seed::new()));

        SEEDS.with(|seeds| {
            Seed(Xoroshiro128Rng::from_seed_u64({
                seeds.borrow_mut().0.gen::<[u64; 2]>()
            }))
        })
    }
}

macro_rules! impl_from_seed {
    ($target:ty) => {
        impl From<Seed> for $target {
            #[inline(always)]
            fn from(seed: Seed) -> $target {
                let mut rng = seed.0;

                rng.gen()
            }
        }
    };
}

impl_from_seed!(u32);
impl_from_seed!(u64);
impl_from_seed!((u64, u64));
impl_from_seed!((u64, u64, u64, u64));
impl_from_seed!([u64; 2]);
impl_from_seed!([u64; 4]);
impl_from_seed!((u128, u128));

impl From<Seed> for u128 {
    #[inline(always)]
    fn from(seed: Seed) -> u128 {
        let mut rng = seed.0;
        let hi = rng.gen::<u64>();
        let lo = rng.gen::<u64>();

        u128::from(hi).wrapping_shl(64) + u128::from(lo)
    }
}

/// `RandomState` provides the default state for `HashMap` or `HashSet` types.
///
/// A particular instance `RandomState` will create the same instances of
/// [`Hasher`], but the hashers created by two different `RandomState`
/// instances are unlikely to produce the same result for the same values.
///
/// ```
/// use std::collections::HashMap;
///
/// use fasthash::RandomState;
/// use fasthash::city::Hash64;
///
/// let s = RandomState::<Hash64>::new();
/// let mut map = HashMap::with_hasher(s);
///
/// assert_eq!(map.insert(37, "a"), None);
/// assert_eq!(map.is_empty(), false);
///
/// map.insert(37, "b");
/// assert_eq!(map.insert(37, "c"), Some("b"));
/// assert_eq!(map[&37], "c");
/// ```
#[derive(Clone)]
pub struct RandomState<T: FastHash> {
    seed: Seed,
    phantom: PhantomData<T>,
}

impl<T: FastHash> RandomState<T> {
    /// Constructs a new `RandomState` that is initialized with random keys.
    #[inline(always)]
    pub fn new() -> Self {
        RandomState {
            seed: Seed::gen(),
            phantom: PhantomData,
        }
    }
}

impl<T: FastHash> BuildHasher for RandomState<T> {
    type Hasher = T::FastHasher;

    #[inline(always)]
    fn build_hasher(&self) -> Self::Hasher {
        T::FastHasher::with_seed(self.seed.into())
    }
}

impl<T: FastHash> Default for RandomState<T> {
    #[inline(always)]
    fn default() -> Self {
        RandomState::new()
    }
}

#[doc(hidden)]
macro_rules! impl_build_hasher {
    ($hasher:ident, $hash:ident) => {
        impl ::std::hash::BuildHasher for $hash {
            type Hasher = $hasher;

            #[inline(always)]
            fn build_hasher(&self) -> Self::Hasher {
                <$hasher as $crate::hasher::FastHasher>::new()
            }
        }

        impl $crate::hasher::BuildHasherExt for $hash {
            type FastHasher = $hasher;
        }
    };
}

impl<T> HasherExt for T
where
    T: TrivialHasher + FastHasher<Output = u128>,
{
    #[inline(always)]
    fn finish_ext(&self) -> u128 {
        self.finalize()
    }
}

#[doc(hidden)]
macro_rules! impl_digest {
    ($hasher:ident, $output:ident) => {
        #[cfg(feature = "digest")]
        impl digest::Digest for $hasher {
            type OutputSize = <$output as crate::hasher::Output>::Size;

            fn new() -> Self {
                Self::default()
            }

            fn input<B: AsRef<[u8]>>(&mut self, data: B) {
                use core::hash::Hasher;

                self.write(data.as_ref());
            }

            fn chain<B: AsRef<[u8]>>(mut self, data: B) -> Self
            where
                Self: Sized,
            {
                self.input(data);
                self
            }

            fn result(self) -> digest::generic_array::GenericArray<u8, Self::OutputSize> {
                use crate::hasher::TrivialHasher;

                self.finalize().to_ne_bytes().into()
            }

            fn result_reset(
                &mut self,
            ) -> digest::generic_array::GenericArray<u8, Self::OutputSize> {
                let result = self.clone().result();
                self.reset();
                result
            }

            fn reset(&mut self) {
                *self = Self::default();
            }

            fn output_size() -> usize {
                core::mem::size_of::<$output>()
            }

            fn digest(data: &[u8]) -> digest::generic_array::GenericArray<u8, Self::OutputSize> {
                Self::default().chain(data).result()
            }
        }
    };
}

cfg_if! {
    if #[cfg(feature = "digest")] {
        /// The `Digest` output type
        pub trait Output {
            /// The `Digest` output size
            type Size;
        }

        impl Output for u32 {
            type Size = digest::generic_array::typenum::U4;
        }

        impl Output for u64 {
            type Size = digest::generic_array::typenum::U8;
        }

        impl Output for u128 {
            type Size = digest::generic_array::typenum::U16;
        }
    }
}

pub trait TrivialHasher: FastHasher {
    fn finalize(&self) -> Self::Output;
}

#[doc(hidden)]
#[macro_export]
macro_rules! trivial_hasher {
    ($(#[$meta:meta])* $hasher:ident ( $hash:ident ) -> $output:ident) => {
        /// An implementation of `std::hash::Hasher`.
        #[derive(Clone, Debug)]
        $(#[$meta])*
        pub struct $hasher {
            seed: Option<<$hash as $crate::hasher::FastHash>::Seed>,
            bytes: Vec<u8>,
        }

        impl Default for $hasher {
            fn default() -> Self {
                <$hasher as $crate::hasher::FastHasher>::new()
            }
        }

        impl $crate::hasher::TrivialHasher for $hasher {
            #[inline(always)]
            fn finalize(&self) -> $output {
                self.seed
                    .map_or_else(
                        || $hash::hash(&self.bytes),
                        |seed| $hash::hash_with_seed(&self.bytes, seed),
                    )
            }
        }

        impl ::std::hash::Hasher for $hasher {
            #[inline(always)]
            fn finish(&self) -> u64 {
                use crate::hasher::TrivialHasher;

                self.finalize() as _
            }

            #[inline(always)]
            fn write(&mut self, bytes: &[u8]) {
                self.bytes.extend_from_slice(bytes)
            }
        }

        impl $crate::hasher::FastHasher for $hasher {
            type Output = $output;
            type Seed = <$hash as $crate::hasher::FastHash>::Seed;

            #[inline(always)]
            fn new() -> Self {
                <Self as $crate::hasher::BufHasher>::with_capacity_and_seed(64, None)
            }

            #[inline(always)]
            fn with_seed(seed: Self::Seed) -> Self {
                <Self as $crate::hasher::BufHasher>::with_capacity_and_seed(64, Some(seed))
            }
        }

        impl ::std::convert::AsRef<[u8]> for $hasher {
            #[inline(always)]
            fn as_ref(&self) -> &[u8] {
                &self.bytes
            }
        }

        impl $crate::hasher::BufHasher for $hasher {
            #[inline(always)]
            fn with_capacity_and_seed(capacity: usize, seed: Option<Self::Seed>) -> Self {
                $hasher {
                    seed,
                    bytes: Vec::with_capacity(capacity),
                }
            }
        }

        impl_build_hasher!($hasher, $hash);
        impl_digest!($hasher, $output);
    };
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::convert::Into;

    use crate::*;

    #[test]
    fn test_seed() {
        let mut s = Seed::new();
        let mut u0: u32 = s.into();
        let mut u1: u64 = s.into();
        let mut u2: u128 = s.into();

        assert!(u0 != 0);
        assert!(u1 != 0);
        assert!(u2 != 0);
        assert_eq!(u0, u1 as u32);
        assert_eq!(u1, (u2 >> 64) as u64);

        s = Seed::gen();

        u1 = s.into();

        s = Seed::gen();

        u2 = s.into();

        assert!(u0 != 0);
        assert!(u1 != 0);
        assert!(u2 != 0);
        assert!(u1 != u0 as u64);
        assert!(u1 != u2 as u64);
        assert!(u1 != (u2 >> 64) as u64);

        u0 = Seed::gen().into();
        u1 = Seed::gen().into();
        u2 = Seed::gen().into();

        assert!(u0 != 0);
        assert!(u1 != 0);
        assert!(u2 != 0);
        assert!(u1 != u0 as u64);
        assert!(u1 != u2 as u64);
        assert!(u1 != (u2 >> 64) as u64);
    }

    macro_rules! test_hashmap_with_fixed_state {
        ($hash:path) => {
            let mut map = HashMap::with_hasher($hash);

            assert_eq!(map.insert(37, "a"), None);
            assert_eq!(map.is_empty(), false);

            map.insert(37, "b");
            assert_eq!(map.insert(37, "c"), Some("b"));
            assert_eq!(map[&37], "c");
        };
    }

    macro_rules! test_hashmap_with_random_state {
        ($hash:path) => {
            let s = RandomState::<$hash>::new();
            let mut map = HashMap::with_hasher(s);

            assert_eq!(map.insert(37, "a"), None);
            assert_eq!(map.is_empty(), false);

            map.insert(37, "b");
            assert_eq!(map.insert(37, "c"), Some("b"));
            assert_eq!(map[&37], "c");
        };
    }

    macro_rules! test_hashmap_with_hashers {
        [ $( $hash:path ),* ] => {
            $( {
                test_hashmap_with_fixed_state!( $hash );
                test_hashmap_with_random_state!( $hash );
            } )*
        }
    }

    #[test]
    fn test_hashmap_with_hashers() {
        #[cfg(feature = "city")]
        test_hashmap_with_hashers![city::Hash32, city::Hash64, city::Hash128];

        #[cfg(all(feature = "city", any(feature = "sse42", target_feature = "sse4.2")))]
        test_hashmap_with_hashers![city::crc::Hash128];

        #[cfg(feature = "farm")]
        test_hashmap_with_hashers![farm::Hash32, farm::Hash64, farm::Hash128];

        #[cfg(feature = "lookup3")]
        test_hashmap_with_hashers![lookup3::Hash32];

        #[cfg(feature = "metro")]
        test_hashmap_with_hashers![
            metro::Hash64_1,
            metro::Hash64_2,
            metro::Hash128_1,
            metro::Hash128_2
        ];

        #[cfg(feature = "metro")]
        #[cfg(any(feature = "sse42", target_feature = "sse4.2"))]
        test_hashmap_with_hashers![
            metro::crc::Hash64_1,
            metro::crc::Hash64_2,
            metro::crc::Hash128_1,
            metro::crc::Hash128_2
        ];

        #[cfg(feature = "mum")]
        test_hashmap_with_hashers![mum::Hash64];

        #[cfg(feature = "murmur")]
        test_hashmap_with_hashers![
            murmur::Hash32,
            murmur::Hash32Aligned,
            murmur2::Hash32,
            murmur2::Hash32A,
            murmur2::Hash32Neutral,
            murmur2::Hash32Aligned,
            murmur2::Hash64_x64,
            murmur2::Hash64_x86,
            murmur3::Hash32,
            murmur3::Hash128_x86,
            murmur3::Hash128_x64
        ];

        #[cfg(feature = "seahash")]
        test_hashmap_with_hashers![sea::Hash64];

        #[cfg(feature = "spooky")]
        test_hashmap_with_hashers![spooky::Hash32, spooky::Hash64, spooky::Hash128];

        #[cfg(feature = "t1ha")]
        test_hashmap_with_hashers![
            t1ha0::Hash64,
            t1ha1::Hash64Le,
            t1ha1::Hash64Be,
            t1ha2::Hash64AtOnce,
            t1ha2::Hash128AtOnce
        ];

        #[cfg(feature = "xx")]
        test_hashmap_with_hashers![xx::Hash32, xx::Hash64];

        #[cfg(feature = "ahash")]
        test_hashmap_with_hashers![ahash::Hash64]
    }
}
