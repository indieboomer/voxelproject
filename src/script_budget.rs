//! Execution guard shared by module initialization and every Lua callback.
use std::cell::Cell;
use std::panic::{catch_unwind, resume_unwind, AssertUnwindSafe};
use std::rc::Rc;
use std::time::{Duration, Instant};

use mlua::{HookTriggers, Lua};

use crate::world_api_gen::{
    SCRIPT_API_CALLS, SCRIPT_HOOK_INTERVAL, SCRIPT_INSTRUCTIONS, SCRIPT_NATIVE_WORK, SCRIPT_TIME_MS,
};

const MAX_SCRIPT_TIME: Duration = Duration::from_millis(SCRIPT_TIME_MS);

#[derive(Clone, Copy, Debug, Default)]
pub struct WorkUsage {
    pub instructions: u32,
    pub api_calls: u32,
    pub native_work: u32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Exhausted {
    Time,
    Instructions,
    ApiCalls,
    Resource(&'static str),
}

impl Exhausted {
    fn message(self) -> &'static str {
        match self {
            Self::Time => "script exceeded its time budget",
            Self::Instructions => "script exceeded its instruction budget",
            Self::ApiCalls => "script exceeded its API call budget",
            Self::Resource(message) => message,
        }
    }
}

pub struct ExecutionBudget {
    start: Cell<Option<Instant>>,
    instructions: Cell<u32>,
    api_calls: Cell<u32>,
    native_work: Cell<u32>,
    exhausted: Cell<Option<Exhausted>>,
    max_time: Duration,
    max_instructions: u32,
}

impl ExecutionBudget {
    pub fn install(lua: &Lua) -> Rc<Self> {
        Self::install_with_limits(lua, MAX_SCRIPT_TIME, SCRIPT_INSTRUCTIONS)
    }

    fn install_with_limits(lua: &Lua, max_time: Duration, max_instructions: u32) -> Rc<Self> {
        let budget = Rc::new(Self {
            start: Cell::new(None),
            instructions: Cell::new(0),
            api_calls: Cell::new(0),
            native_work: Cell::new(0),
            exhausted: Cell::new(None),
            max_time,
            max_instructions,
        });
        let hook_budget = budget.clone();
        lua.set_hook(
            HookTriggers::new().every_nth_instruction(SCRIPT_HOOK_INTERVAL),
            move |_, _| {
                hook_budget.instructions.set(
                    hook_budget
                        .instructions
                        .get()
                        .saturating_add(SCRIPT_HOOK_INTERVAL),
                );
                hook_budget.check();
                Ok(())
            },
        );
        lua.set_app_data(budget.clone());
        budget
    }

    fn abort(&self, reason: Exhausted) -> ! {
        self.exhausted.set(Some(reason));
        // mlua catches this typed unwind at its FFI boundary and propagates
        // it past Lua pcall/xpcall when catch_rust_panics(false) is used.
        // Only run() below consumes our sentinel; unrelated Rust panics
        // still propagate. resume_unwind avoids invoking the panic logger.
        resume_unwind(Box::new(reason))
    }

    pub fn check(&self) {
        if let Some(reason) = self.exhausted.get() {
            self.abort(reason);
        }
        if self.instructions.get() >= self.max_instructions {
            self.abort(Exhausted::Instructions);
        }
        if self
            .start
            .get()
            .is_some_and(|start| start.elapsed() > self.max_time)
        {
            self.abort(Exhausted::Time);
        }
    }

    pub fn charge_api_call(&self) {
        self.check();
        if self.api_calls.get() >= SCRIPT_API_CALLS {
            self.abort(Exhausted::ApiCalls);
        }
        self.api_calls.set(self.api_calls.get() + 1);
    }

    pub fn exhaust(&self, message: &'static str) -> ! {
        self.abort(Exhausted::Resource(message))
    }

    pub fn charge_native_work(&self, units: u32) {
        self.check();
        let next = self.native_work.get().saturating_add(units);
        if next > SCRIPT_NATIVE_WORK {
            self.exhaust("script exceeded its native work budget");
        }
        self.native_work.set(next);
    }

    pub fn usage(&self) -> WorkUsage {
        WorkUsage {
            // Round up the final partial hook interval so admission never
            // undercounts instructions between the last hook and return.
            instructions: self
                .instructions
                .get()
                .saturating_add(SCRIPT_HOOK_INTERVAL)
                .min(self.max_instructions),
            api_calls: self.api_calls.get(),
            native_work: self.native_work.get(),
        }
    }

    pub fn run<T>(&self, call: impl FnOnce() -> mlua::Result<T>) -> mlua::Result<T> {
        self.instructions.set(0);
        self.api_calls.set(0);
        self.native_work.set(0);
        self.exhausted.set(None);
        self.start.set(Some(Instant::now()));
        let result = catch_unwind(AssertUnwindSafe(|| {
            let result = call();
            // A native operation or short script can finish without a
            // further Lua hook; still report a deadline overrun as failure.
            self.check();
            result
        }));
        self.start.set(None);
        match result {
            Ok(result) => result,
            Err(payload) => match payload.downcast::<Exhausted>() {
                Ok(reason) => Err(mlua::Error::RuntimeError(reason.message().into())),
                Err(other) => resume_unwind(other),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instruction_ceiling_is_independent_of_wall_time() {
        let lua = Lua::new_with(
            mlua::StdLib::NONE,
            mlua::LuaOptions::new().catch_rust_panics(false),
        )
        .unwrap();
        let budget = ExecutionBudget::install_with_limits(&lua, Duration::from_secs(60), 1000);
        let error = budget
            .run(|| lua.load("for i = 1, 10000 do end").exec())
            .unwrap_err();
        assert!(error.to_string().contains("instruction budget"));
        budget
            .run(|| lua.load("assert(2 + 2 == 4)").exec())
            .unwrap();
    }

    #[test]
    fn deadline_is_checked_even_without_a_lua_hook() {
        let lua = Lua::new();
        let budget = ExecutionBudget::install_with_limits(&lua, Duration::from_millis(1), u32::MAX);
        let error = budget
            .run(|| {
                std::thread::sleep(Duration::from_millis(5));
                Ok(())
            })
            .unwrap_err();
        assert!(error.to_string().contains("time budget"));
        assert!(budget.start.get().is_none());
    }

    #[test]
    fn unrelated_rust_panics_are_not_misreported_as_script_errors() {
        let lua = Lua::new();
        let budget = ExecutionBudget::install(&lua);
        let panic = catch_unwind(AssertUnwindSafe(|| {
            budget.run::<()>(|| resume_unwind(Box::new(42u32)))
        }))
        .unwrap_err();
        assert_eq!(*panic.downcast::<u32>().unwrap(), 42);
        assert!(budget.start.get().is_none());
    }
}
