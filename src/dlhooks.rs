//! A fork of redhook which helps intercepting libraries

#![cfg(target_os = "linux")]

use libc::{c_char, c_void};

#[link(name = "dl")]
extern "C" {
    fn dlsym(handle: *const c_void, symbol: *const c_char) -> *const c_void;
}

const RTLD_NEXT: *const c_void = -1isize as *const c_void;

pub fn dlsym_next(symbol: &'static str) -> *const u8 {
    let ptr = unsafe { dlsym(RTLD_NEXT, symbol.as_ptr() as *const c_char) };
    if ptr.is_null() {
        panic!("redhook: Unable to find underlying function for {}", symbol);
    }
    ptr as *const u8
}

#[macro_export]
macro_rules! hook {
    (unsafe fn $real_fn:ident ( $($v:ident : $t:ty),* ) -> $r:ty => $body:block) => {
        #[allow(non_camel_case_types)]
        pub struct $real_fn {__private_field: ()}
        #[allow(non_upper_case_globals)]
        static $real_fn: $real_fn = $real_fn {__private_field: ()};

        impl $real_fn {
            unsafe fn hook( $($v : $t),* ) -> $r {
                $body
            }

            pub fn orig() -> unsafe extern "C" fn ( $($v : $t),* ) -> $r {
                static REAL: ::std::sync::OnceLock<usize> = ::std::sync::OnceLock::new();
                let real = *REAL.get_or_init(|| {
                    $crate::dlhooks::dlsym_next(concat!(stringify!($real_fn), "\0")) as usize
                });

                unsafe { ::std::mem::transmute(real) }
            }

            pub fn call_orig( $($v : $t),* ) -> $r {
                unsafe { Self::orig()( $($v),* ) }
            }

            #[no_mangle]
            pub unsafe extern "C" fn $real_fn ( $($v : $t),* ) -> $r {
                ::std::panic::catch_unwind(|| Self::hook( $($v),* )).unwrap_or_else(|_| Self::orig() ( $($v),* ))
            }
        }
    };

    (unsafe fn $real_fn:ident ( $($v:ident : $t:ty),* ) => $body:block) => {
        $crate::hook! { unsafe fn $real_fn ( $($v : $t),* ) -> () => $body }
    };
}

pub(crate) use hook;
