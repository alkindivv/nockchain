// Base field arithmetic functions - Optimized version.

pub const PRIME: u64 = 18446744069414584321;
pub const PRIME_PRIME: u64 = PRIME - 2;
pub const PRIME_128: u128 = 18446744069414584321;
pub const H: u64 = 20033703337;
pub const ORDER: u64 = 2_u64.pow(32);

// Precomputed constants for optimization
const PRIME_MINUS_ONE: u64 = PRIME - 1;
const REDUCTION_MASK: u64 = (1u64 << 32) - 1; // 2^32 - 1

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
              debug_assert!($crate::form::math::base::based_check($x), "element must be inside the field\r");
          )*
      }
    };
}

// Optimized addition with overflow handling
#[inline(always)]
pub fn badd(a: u64, b: u64) -> u64 {
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
pub fn bneg(a: u64) -> u64 {
    based!(a);
    if a != 0 {
        PRIME - a
    } else {
        0
    }
}

// Optimized subtraction
#[inline(always)]
pub fn bsub(a: u64, b: u64) -> u64 {
    based!(a);
    based!(b);

    if a >= b {
        a - b
    } else {
        a.wrapping_add(PRIME - b)
    }
}

/// Optimized reduction for PRIME = 2^64 - 2^32 + 1
#[inline(always)]
pub fn reduce(n: u128) -> u64 {
    let low = n as u64;
    let high = (n >> 64) as u64;

    if high == 0 {
        // Fast path for small numbers
        if low >= PRIME {
            low - PRIME
        } else {
            low
        }
    } else {
        // For PRIME = 2^64 - 2^32 + 1, we can use optimized reduction
        // high * 2^64 ≡ high * (2^32 - 1) (mod PRIME)
        let high_reduced = (high as u128) * (REDUCTION_MASK as u128);
        let total = (low as u128) + high_reduced;

        if total >= PRIME_128 {
            (total - PRIME_128) as u64
        } else {
            total as u64
        }
    }
}

#[inline(always)]
pub fn bmul(a: u64, b: u64) -> u64 {
    based!(a);
    based!(b);
    reduce((a as u128) * (b as u128))
}

// Optimized exponentiation using binary method with better loop structure
#[inline(always)]
pub fn bpow(mut a: u64, mut b: u64) -> u64 {
    based!(a);
    based!(b);

    if b == 0 {
        return 1;
    }
    if b == 1 {
        return a;
    }

    let mut result = 1u64;

    // Binary exponentiation - more efficient than original
    while b > 0 {
        if b & 1 == 1 {
            result = bmul(result, a);
        }
        a = bmul(a, a);
        b >>= 1;
    }

    result
}

#[inline(always)]
pub fn bdiv(a: u64, b: u64) -> u64 {
    bmul(a, binv(b))
}

#[inline(always)]
pub fn binv(a: u64) -> u64 {
    based!(a);
    if a == 0 {
        panic!("Division by zero in field");
    }
    // Due to fermat's little theorem, a^(p-1) = 1 (mod p), so a^(p-2) = a^(-1) (mod p)
    bpow(a, PRIME_PRIME)
}

// Batch operations for better performance when processing multiple elements
pub fn badd_batch(a: &[u64], b: &[u64], result: &mut [u64]) {
    assert_eq!(a.len(), b.len());
    assert_eq!(a.len(), result.len());

    for i in 0..a.len() {
        result[i] = badd(a[i], b[i]);
    }
}

pub fn bmul_batch(a: &[u64], b: &[u64], result: &mut [u64]) {
    assert_eq!(a.len(), b.len());
    assert_eq!(a.len(), result.len());

    for i in 0..a.len() {
        result[i] = bmul(a[i], b[i]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_binv() {
        assert_eq!(bmul(binv(888), 888), 1);
    }

    #[test]
    fn test_optimized_operations() {
        let a = 12345678901234567890u64 % PRIME;
        let b = 98765432109876543210u64 % PRIME;

        // Test basic operations
        let sum = badd(a, b);
        let diff = bsub(a, b);
        let prod = bmul(a, b);
        let pow = bpow(a, 17);
        let inv = binv(a);

        // Verify properties
        assert_eq!(bmul(a, inv), 1);
        assert_eq!(badd(diff, b), a);
        assert!(sum < PRIME);
        assert!(diff < PRIME);
        assert!(prod < PRIME);
        assert!(pow < PRIME);
    }

    #[test]
    fn test_reduce() {
        let large_num = (PRIME as u128) * 5 + 123;
        let reduced = reduce(large_num);
        assert!(reduced < PRIME);
        assert_eq!(reduced, 123);
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
