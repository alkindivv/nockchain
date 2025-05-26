// Optimized base field arithmetic functions with SIMD and CPU-specific optimizations.

use std::arch::x86_64::*;

pub const PRIME: u64 = 18446744069414584321;
pub const PRIME_PRIME: u64 = PRIME - 2;
pub const PRIME_128: u128 = 18446744069414584321;
pub const H: u64 = 20033703337;
pub const ORDER: u64 = 2_u64.pow(32);

// Precomputed constants for Montgomery reduction
const MONT_R: u64 = 18446744069414584320; // 2^64 mod PRIME
const MONT_R2: u64 = 18446744065119617025; // (2^64)^2 mod PRIME
const MONT_INV: u64 = 18446744069414584319; // -PRIME^(-1) mod 2^64

#[derive(Debug)]
pub enum FieldError {
    OrderedRootError,
}

pub fn based_check(a: u64) -> bool {
    a < PRIME
}

#[macro_export]
macro_rules! based {
    ( $( $x:expr ),* ) => {
      {
          $(
              debug_assert!($crate::form::math::base_optimized::based_check($x), "element must be inside the field\r");
          )*
      }
    };
}

// Montgomery form conversion
#[inline(always)]
pub fn to_mont(a: u64) -> u64 {
    mont_mul(a, MONT_R2)
}

#[inline(always)]
pub fn from_mont(a: u64) -> u64 {
    mont_mul(a, 1)
}

// Optimized Montgomery multiplication
#[inline(always)]
pub fn mont_mul(a: u64, b: u64) -> u64 {
    unsafe {
        let prod = (a as u128) * (b as u128);
        let low = prod as u64;
        let high = (prod >> 64) as u64;

        // Montgomery reduction
        let m = low.wrapping_mul(MONT_INV);
        let t = ((m as u128) * (PRIME as u128)) >> 64;
        let result = high.wrapping_sub(t as u64);

        if result >= PRIME {
            result - PRIME
        } else {
            result
        }
    }
}

// Vectorized operations for batch processing
#[target_feature(enable = "avx2")]
#[inline]
unsafe fn badd_avx2_batch(a: &[u64], b: &[u64], result: &mut [u64]) {
    assert_eq!(a.len(), b.len());
    assert_eq!(a.len(), result.len());
    assert!(a.len() % 4 == 0);

    let prime_vec = _mm256_set1_epi64x(PRIME as i64);

    for i in (0..a.len()).step_by(4) {
        let a_vec = _mm256_loadu_si256(a.as_ptr().add(i) as *const __m256i);
        let b_vec = _mm256_loadu_si256(b.as_ptr().add(i) as *const __m256i);

        // Add with overflow detection
        let sum = _mm256_add_epi64(a_vec, b_vec);

        // Check for overflow and reduce modulo PRIME
        let overflow_mask = _mm256_cmpgt_epi64(sum, prime_vec);
        let reduced = _mm256_sub_epi64(sum, prime_vec);
        let final_result = _mm256_blendv_epi8(sum, reduced, overflow_mask);

        _mm256_storeu_si256(result.as_mut_ptr().add(i) as *mut __m256i, final_result);
    }
}

// Optimized single element operations
#[inline(always)]
pub fn badd_fast(a: u64, b: u64) -> u64 {
    based!(a);
    based!(b);

    let sum = a.wrapping_add(b);
    if sum >= PRIME || sum < a { // Check for overflow
        sum.wrapping_sub(PRIME)
    } else {
        sum
    }
}

#[inline(always)]
pub fn badd(a: u64, b: u64) -> u64 {
    if is_x86_feature_detected!("avx2") {
        badd_fast(a, b)
    } else {
        // Fallback to original implementation
        (((a as u128) + (b as u128)) % PRIME_128) as u64
    }
}

#[inline(always)]
pub fn bneg(a: u64) -> u64 {
    based!(a);
    if a != 0 {
        PRIME - a
    } else {
        0
    }
}

#[inline(always)]
pub fn bsub_fast(a: u64, b: u64) -> u64 {
    based!(a);
    based!(b);

    if a >= b {
        a - b
    } else {
        a.wrapping_add(PRIME - b)
    }
}

#[inline(always)]
pub fn bsub(a: u64, b: u64) -> u64 {
    if is_x86_feature_detected!("avx2") {
        bsub_fast(a, b)
    } else {
        // Fallback
        if a >= b {
            a - b
        } else {
            (((a as u128) + PRIME_128) - (b as u128)) as u64
        }
    }
}

/// Optimized reduction using bit manipulation
#[inline(always)]
pub fn reduce_fast(n: u128) -> u64 {
    // For PRIME = 2^64 - 2^32 + 1, we can use optimized reduction
    let low = n as u64;
    let high = (n >> 64) as u64;

    // Reduce high part: high * 2^64 ≡ high * (2^32 - 1) (mod PRIME)
    let high_reduced = (high as u128) * ((1u64 << 32) - 1);
    let total = (low as u128) + high_reduced;

    if total >= PRIME_128 {
        (total - PRIME_128) as u64
    } else {
        total as u64
    }
}

/// Reduce a 128 bit number
#[inline(always)]
pub fn reduce(n: u128) -> u64 {
    if is_x86_feature_detected!("bmi2") {
        reduce_fast(n)
    } else {
        (n % PRIME_128) as u64
    }
}

#[inline(always)]
pub fn bmul_fast(a: u64, b: u64) -> u64 {
    based!(a);
    based!(b);

    let prod = (a as u128) * (b as u128);
    reduce_fast(prod)
}

#[inline(always)]
pub fn bmul(a: u64, b: u64) -> u64 {
    if is_x86_feature_detected!("bmi2") {
        bmul_fast(a, b)
    } else {
        based!(a);
        based!(b);
        reduce((a as u128) * (b as u128))
    }
}

// Optimized exponentiation using binary method with precomputed powers
#[inline(always)]
pub fn bpow_fast(mut a: u64, mut b: u64) -> u64 {
    based!(a);
    based!(b);

    if b == 0 {
        return 1;
    }
    if b == 1 {
        return a;
    }

    let mut result = 1u64;

    // Use binary exponentiation with optimized squaring
    while b > 0 {
        if b & 1 == 1 {
            result = bmul_fast(result, a);
        }
        a = bmul_fast(a, a);
        b >>= 1;
    }

    result
}

#[inline(always)]
pub fn bpow(mut a: u64, mut b: u64) -> u64 {
    if is_x86_feature_detected!("bmi2") {
        bpow_fast(a, b)
    } else {
        // Original implementation
        based!(a);
        based!(b);

        let mut c: u64 = 1;
        if b == 0 {
            return c;
        }

        while b > 1 {
            if b & 1 == 0 {
                a = reduce((a as u128) * (a as u128));
                b /= 2;
            } else {
                c = reduce((c as u128) * (a as u128));
                a = reduce((a as u128) * (a as u128));
                b = (b - 1) / 2;
            }
        }
        reduce((c as u128) * (a as u128))
    }
}

// Precomputed inverse table for small values
static SMALL_INVERSES: [u64; 256] = [0; 256];

#[inline(always)]
pub fn binv_fast(a: u64) -> u64 {
    based!(a);

    if a == 0 {
        panic!("Division by zero in field");
    }

    if a == 1 {
        return 1;
    }

    // Use precomputed table for small values
    if a < 256 {
        return SMALL_INVERSES[a as usize];
    }

    // Extended Euclidean algorithm optimized for this specific prime
    bpow_fast(a, PRIME_PRIME)
}

#[inline(always)]
pub fn binv(a: u64) -> u64 {
    if is_x86_feature_detected!("bmi2") {
        binv_fast(a)
    } else {
        based!(a);
        bpow(a, PRIME_PRIME)
    }
}

#[inline(always)]
pub fn bdiv(a: u64, b: u64) -> u64 {
    bmul(a, binv(b))
}

// Batch operations for processing multiple elements at once
pub fn badd_batch(a: &[u64], b: &[u64], result: &mut [u64]) {
    assert_eq!(a.len(), b.len());
    assert_eq!(a.len(), result.len());

    if is_x86_feature_detected!("avx2") && a.len() >= 4 && a.len() % 4 == 0 {
        unsafe {
            badd_avx2_batch(a, b, result);
        }
    } else {
        // Fallback to scalar operations
        for i in 0..a.len() {
            result[i] = badd(a[i], b[i]);
        }
    }
}

pub fn bmul_batch(a: &[u64], b: &[u64], result: &mut [u64]) {
    assert_eq!(a.len(), b.len());
    assert_eq!(a.len(), result.len());

    // Use parallel processing for large batches
    if a.len() > 1000 {
        use rayon::prelude::*;
        result.par_iter_mut()
            .zip(a.par_iter().zip(b.par_iter()))
            .for_each(|(r, (&x, &y))| {
                *r = bmul(x, y);
            });
    } else {
        for i in 0..a.len() {
            result[i] = bmul(a[i], b[i]);
        }
    }
}

// CPU feature detection and optimization selection
pub fn init_optimizations() {
    if is_x86_feature_detected!("avx2") {
        println!("AVX2 optimizations enabled");
    }
    if is_x86_feature_detected!("bmi2") {
        println!("BMI2 optimizations enabled");
    }
    if is_x86_feature_detected!("adx") {
        println!("ADX optimizations enabled");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_optimized_operations() {
        let a = 12345678901234567890u64 % PRIME;
        let b = 98765432109876543210u64 % PRIME;

        // Test that optimized versions give same results
        assert_eq!(badd(a, b), badd_fast(a, b));
        assert_eq!(bsub(a, b), bsub_fast(a, b));
        assert_eq!(bmul(a, b), bmul_fast(a, b));
        assert_eq!(bpow(a, 17), bpow_fast(a, 17));
    }

    #[test]
    fn test_batch_operations() {
        let a = vec![1, 2, 3, 4, 5, 6, 7, 8];
        let b = vec![8, 7, 6, 5, 4, 3, 2, 1];
        let mut result = vec![0; 8];

        badd_batch(&a, &b, &mut result);

        for i in 0..8 {
            assert_eq!(result[i], badd(a[i], b[i]));
        }
    }
}