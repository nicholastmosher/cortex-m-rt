//! Minimal startup / runtime for Cortex-M microcontrollers
//!
//! # Features
//!
//! This crate provides
//!
//! - Before main initialization of the `.bss` and `.data` sections
//!
//! - An overridable (\*) `panic_fmt` implementation that prints to the ITM or
//!   to the host stdout (through semihosting) depending on which Cargo feature
//!   has been enabled: `"panic-over-itm"` or `"panic-over-semihosting"`.
//!
//! - A minimal `start` lang item, to support vanilla `fn main()`. NOTE the
//!   processor goes into "reactive" mode (`loop { asm!("wfi") }`) after
//!   returning from `main`.
//!
//! - An opt-in linker script (`"linker-script"` Cargo feature) that encodes
//!   the memory layout of a generic Cortex-M microcontroller. This linker
//!   script is missing the definitions of the FLASH and RAM memory regions of
//!   the device and of the `_stack_start` symbol (address where the call stack
//!   is allocated). This missing information must be supplied through a
//!   `memory.x` file (see example below).
//!
//! - A default exception handler tailored for debugging and that provides
//!   access to the stacked registers under the debugger. By default, all
//!   exceptions (\*\*) are serviced by this handler but this can be overridden
//!   on a per exception basis by opting out of the "exceptions" Cargo feature
//!   and then defining the following `struct`
//!
//! - A `_sheap` symbol at whose address you can locate the heap.
//!
//! ``` ignore,no_run
//! use cortex_m::exception;
//!
//! #[link_section = ".rodata.exceptions"]
//! #[used]
//! static EXCEPTIONS: exception::Handlers = exception::Handlers {
//!     hard_fault: my_override,
//!     nmi: another_handler,
//!     ..exception::DEFAULT_HANDLERS
//! };
//! ```
//!
//! (\*) To override the `panic_fmt` implementation, simply create a new
//! `rust_begin_unwind` symbol:
//!
//! ```
//! #[no_mangle]
//! pub unsafe extern "C" fn rust_begin_unwind(
//!     _args: ::core::fmt::Arguments,
//!     _file: &'static str,
//!     _line: u32,
//! ) -> ! {
//!     ..
//! }
//! ```
//!
//! (\*\*) All the device specific exceptions, i.e. the interrupts, are left
//! unpopulated. You must fill that part of the vector table by defining the
//! following static (with the right memory layout):
//!
//! ``` ignore,no_run
//! #[link_section = ".rodata.interrupts"]
//! #[used]
//! static INTERRUPTS: SomeStruct = SomeStruct { .. }
//! ```
//!
//! # Example
//!
//! ``` text
//! $ cargo new --bin app && cd $_
//!
//! $ cargo add cortex-m cortex-m-rt
//!
//! $ edit Xargo.toml && cat $_
//! ```
//!
//! ``` text
//! [dependencies.core]
//!
//! [dependencies.compiler_builtins]
//! features = ["mem"]
//! git = "https://github.com/rust-lang-nursery/compiler-builtins"
//! stage = 1
//! ```
//!
//! ``` text
//! $ edit memory.x && cat $_
//! ```
//!
//! ``` text
//! MEMORY
//! {
//!   /* NOTE K = KiBi = 1024 bytes */
//!   FLASH : ORIGIN = 0x08000000, LENGTH = 128K
//!   RAM : ORIGIN = 0x20000000, LENGTH = 8K
//! }
//!
//! /* This is where the call stack will be allocated */
//! _stack_start = ORIGIN(RAM) + LENGTH(RAM);
//! ```
//!
//! ``` text
//! $ edit src/main.rs && cat $_
//! ```
//!
//! ``` ignore,no_run
//! #![feature(used)]
//! #![no_std]
//!
//! #[macro_use]
//! extern crate cortex_m;
//! extern crate cortex_m_rt;
//!
//! use cortex_m::asm;
//!
//! fn main() {
//!     hprintln!("Hello, world!");
//! }
//!
//! // As we are not using interrupts, we just register a dummy catch all
//! // handler
//! #[allow(dead_code)]
//! #[link_section = ".rodata.interrupts"]
//! #[used]
//! static INTERRUPTS: [extern "C" fn(); 240] = [default_handler; 240];
//!
//! extern "C" fn default_handler() {
//!     asm::bkpt();
//! }
//! ```
//!
//! ``` text
//! $ cargo install xargo
//!
//! $ xargo rustc --target thumbv7m-none-eabi -- \
//!       -C link-arg=-Tlink.x -C linker=arm-none-eabi-ld -Z linker-flavor=ld
//!
//! $ arm-none-eabi-objdump -Cd $(find target -name app) | head
//!
//! Disassembly of section .text:
//!
//! 08000400 <cortex_m_rt::reset_handler>:
//!  8000400:       b580            push    {r7, lr}
//!  8000402:       466f            mov     r7, sp
//!  8000404:       b084            sub     sp, #16
//! ```

#![cfg_attr(target_arch = "arm", feature(core_intrinsics))]
#![deny(missing_docs)]
#![deny(warnings)]
#![feature(asm)]
#![feature(compiler_builtins_lib)]
#![feature(lang_items)]
#![feature(linkage)]
#![feature(naked_functions)]
#![feature(used)]
#![no_std]

#[cfg_attr(feature = "panic-over-itm", macro_use)]
extern crate cortex_m;
extern crate compiler_builtins;
#[cfg(feature = "panic-over-semihosting")]
#[macro_use]
extern crate cortex_m_semihosting;
extern crate r0;

mod lang_items;

#[cfg(target_arch = "arm")]
use cortex_m::exception::StackedRegisters;

#[cfg(target_arch = "arm")]
extern "C" {
    // NOTE `rustc` forces this signature on us. See `src/lang_items.rs`
    fn main(argc: isize, argv: *const *const u8) -> isize;

    // Boundaries of the .bss section
    static mut _ebss: u32;
    static mut _sbss: u32;

    // Boundaries of the .data section
    static mut _edata: u32;
    static mut _sdata: u32;

    // Initial values of the .data section (stored in Flash)
    static _sidata: u32;
}

#[cfg(target_arch = "arm")]
#[used]
#[link_section = ".vector_table.reset_handler"]
static RESET_HANDLER: unsafe extern "C" fn() -> ! = reset_handler;

/// The reset handler
///
/// This is the entry point of all programs
#[cfg(target_arch = "arm")]
#[link_section = ".reset_handler"]
unsafe extern "C" fn reset_handler() -> ! {
    ::r0::zero_bss(&mut _sbss, &mut _ebss);
    ::r0::init_data(&mut _sdata, &mut _edata, &_sidata);

    // Neither `argc` or `argv` make sense in bare metal context so we just
    // stub them
    main(0, ::core::ptr::null());

    // If `main` returns, then we go into "reactive" mode and simply attend
    // interrupts as they occur.
    loop {
        asm!("wfi" :::: "volatile");
    }
}

extern "C" {
    fn BUS_FAULT();
    fn HARD_FAULT();
    fn MEM_MANAGE();
    fn NMI();
    fn PENDSV();
    fn SVCALL();
    fn SYS_TICK();
    fn USAGE_FAULT();
}

#[used]
#[link_section = ".vector_table.exceptions"]
static EXCEPTIONS: [Option<unsafe extern "C" fn()>; 14] = [
    Some(NMI),
    Some(HARD_FAULT),
    Some(MEM_MANAGE),
    Some(BUS_FAULT),
    Some(USAGE_FAULT),
    None,
    None,
    None,
    None,
    Some(SVCALL),
    None,
    None,
    Some(PENDSV),
    Some(SYS_TICK),
];

// This is the actual exception handler. `_sr` is a pointer to the previous
// stack frame
#[cfg(target_arch = "arm")]
extern "C" fn default_handler(_sr: &StackedRegisters) -> ! {
    use cortex_m::exception::Vector;

    // What exception is currently being serviced
    let _v = Vector::active();

    cortex_m::asm::bkpt();

    loop {}
}

#[cfg(target_arch = "arm")]
#[doc(hidden)]
#[export_name = "DEFAULT_HANDLER"]
#[linkage = "weak"]
#[naked]
pub unsafe extern "C" fn _default_handler() -> ! {
    // "trampoline" to get to the real exception handler.
    asm!("mrs r0, MSP
            ldr r1, [r0, #20]
            b $0"
            :
            : "i"(default_handler as extern "C" fn(&StackedRegisters) -> !)
            :
            : "volatile");

    ::core::intrinsics::unreachable()
}
