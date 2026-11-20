#[macro_export]
macro_rules! link { ($lib:literal $abi:literal fn $name:ident($($a:ident: $t:ty),*) -> $r:ty) => {
    #[allow(non_snake_case, unused)] unsafe fn $name($($a: $t),*) -> $r { unimplemented!() }
}; }
pub mod core {
    pub use crate::link;
    #[derive(Clone, Copy)] pub struct GUID(pub u128);
    impl GUID { pub const fn from_u128(v: u128) -> Self { GUID(v) } }
}
