use std::collections::HashMap;
use std::slice::IterMut;
use std::error::Error;
use std::any::Any;
use std::boxed::Box;
use std::fmt;

use parser::{lex, parse, Expr, Stmt, FnDef};
use fn_register::FnRegister;

#[cfg(feature = "modules")]
use module::{Module, ModuleError};

use std::ops::{Add, Sub, Mul, Div, Neg, BitAnd, BitOr, BitXor, Shl, Shr, Rem};
use std::cmp::{PartialOrd, PartialEq};


#[derive(Debug)]
pub enum EvalAltResult {
    ErrorFunctionNotFound,
    ErrorFunctionArgMismatch,
    ErrorFunctionCallNotSupported,
    ErrorIndexMismatch,
    ErrorIfGuardMismatch,
    ErrorVariableNotFound(String),
    ErrorFunctionArityNotSupported,
    ErrorAssignmentToUnknownLHS,
    ErrorMismatchOutputType,
    ErrorCantOpenScriptFile,
    InternalErrorMalformedDotExpression,
    LoopBreak,
    Return(Box<Any>),

    #[cfg(feature = "modules")]
    ModuleError(ModuleError),
    #[cfg(feature = "modules")]
    ErrorModuleMemberNotFound,
    #[cfg(feature = "modules")]
    ErrorErroneousModule,
    #[cfg(feature = "modules")]
    ErrorModuleNotFound,
    #[cfg(feature = "modules")]
    ErrorNotAModule,
}

impl Error for EvalAltResult {
    fn description(&self) -> &str {
        match *self {
            EvalAltResult::ErrorFunctionNotFound => "Function not found",
            EvalAltResult::ErrorFunctionArgMismatch => "Function argument types do not match",
            EvalAltResult::ErrorFunctionCallNotSupported => {
                "Function call with > 2 argument not supported"
            }
            EvalAltResult::ErrorIndexMismatch => "Index does not match array",
            EvalAltResult::ErrorIfGuardMismatch => "If guards expect boolean expression",
            EvalAltResult::ErrorVariableNotFound(_) => "Variable not found",
            EvalAltResult::ErrorFunctionArityNotSupported => {
                "Functions of more than 3 parameters are not yet supported"
            }
            EvalAltResult::ErrorAssignmentToUnknownLHS => {
                "Assignment to an unsupported left-hand side"
            }
            EvalAltResult::ErrorMismatchOutputType => "Cast of output failed",
            EvalAltResult::ErrorCantOpenScriptFile => "Cannot open script file",
            EvalAltResult::InternalErrorMalformedDotExpression => {
                "[Internal error] Unexpected expression in dot expression"
            }
            EvalAltResult::LoopBreak => "Loop broken before completion (not an error)",
            EvalAltResult::Return(_) => "Function returned value (not an error)",

            #[cfg(feature = "modules")]
            EvalAltResult::ModuleError(_) => "module error",
            #[cfg(feature = "modules")]
            EvalAltResult::ErrorErroneousModule => "Module contains erroneous code",
            #[cfg(feature = "modules")]
            EvalAltResult::ErrorModuleMemberNotFound => "Module doesn't contain member",
            #[cfg(feature = "modules")]
            EvalAltResult::ErrorModuleNotFound => "Module not found",
            #[cfg(feature = "modules")]
            EvalAltResult::ErrorNotAModule => "Symbol isn't a module",
        }
    }

    fn cause(&self) -> Option<&Error> {
        None
    }
}

impl fmt::Display for EvalAltResult {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.description())
    }
}

pub enum FnType {
    ExternalFn0(Box<Fn() -> Result<Box<Any>, EvalAltResult>>),
    ExternalFn1(Box<Fn(&mut Box<Any>) -> Result<Box<Any>, EvalAltResult>>),
    ExternalFn2(Box<Fn(&mut Box<Any>, &mut Box<Any>) -> Result<Box<Any>, EvalAltResult>>),
    ExternalFn3(Box<Fn(&mut Box<Any>, &mut Box<Any>, &mut Box<Any>)
                       -> Result<Box<Any>, EvalAltResult>>),
    ExternalFn4(Box<Fn(&mut Box<Any>, &mut Box<Any>, &mut Box<Any>, &mut Box<Any>)
                       -> Result<Box<Any>, EvalAltResult>>),
    ExternalFn5(Box<Fn(&mut Box<Any>,
                       &mut Box<Any>,
                       &mut Box<Any>,
                       &mut Box<Any>,
                       &mut Box<Any>)
                       -> Result<Box<Any>, EvalAltResult>>),
    ExternalFn6(Box<Fn(&mut Box<Any>,
                       &mut Box<Any>,
                       &mut Box<Any>,
                       &mut Box<Any>,
                       &mut Box<Any>,
                       &mut Box<Any>)
                       -> Result<Box<Any>, EvalAltResult>>),
    InternalFn(FnDef),
}

/// Rhai's engine type. This is what you use to run Rhai scripts
///
/// ```rust
/// extern crate rhai;
/// use rhai::Engine;
///
/// fn main() {
///     let mut engine = Engine::new();
///
///     if let Ok(result) = engine.eval::<i64>("40 + 2") {
///         println!("Answer: {}", result);  // prints 42
///     }
/// }
/// ```
pub struct Engine {
    /// A hashmap containing all functions know to the engine
    pub fns: HashMap<String, Vec<FnType>>,
    pub module_register: Option<fn(&mut Engine)>,
}

/// A type containing information about current scope.
/// Useful for keeping state between `Engine` runs
///
/// ```rust
/// use rhai::{Engine, Scope};
///
/// let mut engine = Engine::new();
/// let mut my_scope = Scope::new();
///
/// assert!(engine.eval_with_scope::<()>(&mut my_scope, "let x = 5;").is_ok());
/// assert_eq!(engine.eval_with_scope::<i64>(&mut my_scope, "x + 1").unwrap(), 6);
/// ```
///
/// Between runs, `Engine` only remembers functions when not using own `Scope`.
#[derive(Debug)]
pub struct Scope {
    pub symbols: Vec<(String, Box<Any>)>,
    pub uses: Vec<(String, String, UseType)>,
}

#[derive(Debug, PartialEq, Clone)]
pub enum UseType {
    Function,
    Symbol,
}

impl Scope {
    pub fn new() -> Scope                                     { Scope { symbols: Vec::new(), uses: Vec::new() } }
    pub fn len(&self) -> usize                                { self.symbols.len() }
    pub fn is_empty(&self) -> bool                            { self.symbols.is_empty() }
    pub fn push(&mut self, symbol: (String, Box<Any>))        { self.symbols.push(symbol) }
    pub fn pop(&mut self) -> Option<(String, Box<Any>)>       { self.symbols.pop() }
    pub fn iter_mut(&mut self) -> IterMut<(String, Box<Any>)> { self.symbols.iter_mut() }
}

impl Engine {
    /// Universal method for calling functions, that are either
    /// registered with the `Engine` or written in Rhai
    pub fn call_fn(&self,
               name: &str,
               arg1: Option<&mut Box<Any>>,
               arg2: Option<&mut Box<Any>>,
               arg3: Option<&mut Box<Any>>,
               arg4: Option<&mut Box<Any>>,
               arg5: Option<&mut Box<Any>>,
               arg6: Option<&mut Box<Any>>)
               -> Result<Box<Any>, EvalAltResult> {

        match self.fns.get(name) {
            Some(vf) => {
                match (arg1, arg2, arg3, arg4, arg5, arg6) {
                    (Some(ref mut a1),
                     Some(ref mut a2),
                     Some(ref mut a3),
                     Some(ref mut a4),
                     Some(ref mut a5),
                     Some(ref mut a6)) => {
                        for arr_f in vf {
                            match *arr_f {
                                FnType::ExternalFn6(ref f) => {
                                    if let Ok(v) = f(*a1, *a2, *a3, *a4, *a5, *a6) {
                                        return Ok(v);
                                    }
                                }
                                FnType::InternalFn(ref f) => {
                                    if f.params.len() != 6 {
                                        return Err(EvalAltResult::ErrorFunctionArgMismatch);
                                    }

                                    let mut new_scope: Scope = Scope::new();
                                    let result1 = self.call_fn("clone",
                                                               Some(a1),
                                                               None,
                                                               None,
                                                               None,
                                                               None,
                                                               None);
                                    let result2 = self.call_fn("clone",
                                                               Some(a2),
                                                               None,
                                                               None,
                                                               None,
                                                               None,
                                                               None);
                                    let result3 = self.call_fn("clone",
                                                               Some(a3),
                                                               None,
                                                               None,
                                                               None,
                                                               None,
                                                               None);
                                    let result4 = self.call_fn("clone",
                                                               Some(a4),
                                                               None,
                                                               None,
                                                               None,
                                                               None,
                                                               None);
                                    let result5 = self.call_fn("clone",
                                                               Some(a5),
                                                               None,
                                                               None,
                                                               None,
                                                               None,
                                                               None);
                                    let result6 = self.call_fn("clone",
                                                               Some(a6),
                                                               None,
                                                               None,
                                                               None,
                                                               None,
                                                               None);

                                    match (result1, result2, result3, result4, result5, result6) {
                                        (Ok(r1), Ok(r2), Ok(r3), Ok(r4), Ok(r5), Ok(r6)) => {
                                            new_scope.push((f.params[0].clone(), r1));
                                            new_scope.push((f.params[1].clone(), r2));
                                            new_scope.push((f.params[2].clone(), r3));
                                            new_scope.push((f.params[3].clone(), r4));
                                            new_scope.push((f.params[4].clone(), r5));
                                            new_scope.push((f.params[5].clone(), r6));
                                        }
                                        _ => return Err(EvalAltResult::ErrorFunctionArgMismatch),
                                    }
                                    match self.eval_stmt(&mut new_scope, &*f.body) {
                                        Err(EvalAltResult::Return(x)) => return Ok(x),
                                        x => return x,
                                    }
                                }
                                _ => (),
                            }
                        }
                        Err(EvalAltResult::ErrorFunctionArgMismatch)
                    }
                    (Some(ref mut a1),
                     Some(ref mut a2),
                     Some(ref mut a3),
                     Some(ref mut a4),
                     Some(ref mut a5),
                     None) => {
                        for arr_f in vf {
                            match *arr_f {
                                FnType::ExternalFn5(ref f) => {
                                    if let Ok(v) = f(*a1, *a2, *a3, *a4, *a5) {
                                        return Ok(v);
                                    }
                                }
                                FnType::InternalFn(ref f) => {
                                    if f.params.len() != 5 {
                                        return Err(EvalAltResult::ErrorFunctionArgMismatch);
                                    }

                                    let mut new_scope: Scope = Scope::new();
                                    let result1 = self.call_fn("clone",
                                                               Some(a1),
                                                               None,
                                                               None,
                                                               None,
                                                               None,
                                                               None);
                                    let result2 = self.call_fn("clone",
                                                               Some(a2),
                                                               None,
                                                               None,
                                                               None,
                                                               None,
                                                               None);
                                    let result3 = self.call_fn("clone",
                                                               Some(a3),
                                                               None,
                                                               None,
                                                               None,
                                                               None,
                                                               None);
                                    let result4 = self.call_fn("clone",
                                                               Some(a4),
                                                               None,
                                                               None,
                                                               None,
                                                               None,
                                                               None);
                                    let result5 = self.call_fn("clone",
                                                               Some(a5),
                                                               None,
                                                               None,
                                                               None,
                                                               None,
                                                               None);

                                    match (result1, result2, result3, result4, result5) {
                                        (Ok(r1), Ok(r2), Ok(r3), Ok(r4), Ok(r5)) => {
                                            new_scope.push((f.params[0].clone(), r1));
                                            new_scope.push((f.params[1].clone(), r2));
                                            new_scope.push((f.params[2].clone(), r3));
                                            new_scope.push((f.params[3].clone(), r4));
                                            new_scope.push((f.params[4].clone(), r5));
                                        }
                                        _ => return Err(EvalAltResult::ErrorFunctionArgMismatch),
                                    }
                                    match self.eval_stmt(&mut new_scope, &*f.body) {
                                        Err(EvalAltResult::Return(x)) => return Ok(x),
                                        x => return x,
                                    }
                                }
                                _ => (),
                            }
                        }
                        Err(EvalAltResult::ErrorFunctionArgMismatch)
                    }
                    (Some(ref mut a1),
                     Some(ref mut a2),
                     Some(ref mut a3),
                     Some(ref mut a4),
                     None,
                     None) => {
                        for arr_f in vf {
                            match *arr_f {
                                FnType::ExternalFn4(ref f) => {
                                    if let Ok(v) = f(*a1, *a2, *a3, *a4) {
                                        return Ok(v)
                                    }
                                }
                                FnType::InternalFn(ref f) => {
                                    if f.params.len() != 4 {
                                        return Err(EvalAltResult::ErrorFunctionArgMismatch);
                                    }

                                    let mut new_scope: Scope = Scope::new();
                                    let result1 = self.call_fn("clone",
                                                               Some(a1),
                                                               None,
                                                               None,
                                                               None,
                                                               None,
                                                               None);
                                    let result2 = self.call_fn("clone",
                                                               Some(a2),
                                                               None,
                                                               None,
                                                               None,
                                                               None,
                                                               None);
                                    let result3 = self.call_fn("clone",
                                                               Some(a3),
                                                               None,
                                                               None,
                                                               None,
                                                               None,
                                                               None);
                                    let result4 = self.call_fn("clone",
                                                               Some(a4),
                                                               None,
                                                               None,
                                                               None,
                                                               None,
                                                               None);
                                    match (result1, result2, result3, result4) {
                                        (Ok(r1), Ok(r2), Ok(r3), Ok(r4)) => {
                                            new_scope.push((f.params[0].clone(), r1));
                                            new_scope.push((f.params[1].clone(), r2));
                                            new_scope.push((f.params[2].clone(), r3));
                                            new_scope.push((f.params[3].clone(), r4));
                                        }
                                        _ => return Err(EvalAltResult::ErrorFunctionArgMismatch),
                                    }
                                    match self.eval_stmt(&mut new_scope, &*f.body) {
                                        Err(EvalAltResult::Return(x)) => return Ok(x),
                                        x => return x,
                                    }
                                }
                                _ => (),
                            }
                        }
                        Err(EvalAltResult::ErrorFunctionArgMismatch)
                    }
                    (Some(ref mut a1), Some(ref mut a2), Some(ref mut a3), None, None, None) => {
                        for arr_f in vf {
                            match *arr_f {
                                FnType::ExternalFn3(ref f) => {
                                    if let Ok(v) = f(*a1, *a2, *a3) {
                                        return Ok(v);
                                    }
                                }
                                FnType::InternalFn(ref f) => {
                                    if f.params.len() != 3 {
                                        return Err(EvalAltResult::ErrorFunctionArgMismatch);
                                    }

                                    let mut new_scope: Scope = Scope::new();
                                    let result1 = self.call_fn("clone",
                                                               Some(a1),
                                                               None,
                                                               None,
                                                               None,
                                                               None,
                                                               None);
                                    let result2 = self.call_fn("clone",
                                                               Some(a2),
                                                               None,
                                                               None,
                                                               None,
                                                               None,
                                                               None);
                                    let result3 = self.call_fn("clone",
                                                               Some(a3),
                                                               None,
                                                               None,
                                                               None,
                                                               None,
                                                               None);
                                    match (result1, result2, result3) {
                                        (Ok(r1), Ok(r2), Ok(r3)) => {
                                            new_scope.push((f.params[0].clone(), r1));
                                            new_scope.push((f.params[1].clone(), r2));
                                            new_scope.push((f.params[2].clone(), r3));
                                        }
                                        _ => return Err(EvalAltResult::ErrorFunctionArgMismatch),
                                    }
                                    match self.eval_stmt(&mut new_scope, &*f.body) {
                                        Err(EvalAltResult::Return(x)) => return Ok(x),
                                        x => return x,
                                    }
                                }
                                _ => (),
                            }
                        }
                        Err(EvalAltResult::ErrorFunctionArgMismatch)
                    }
                    (Some(ref mut a1), Some(ref mut a2), None, None, None, None) => {
                        for arr_f in vf {
                            match *arr_f {
                                FnType::ExternalFn2(ref f) => {
                                    if let Ok(v) = f(*a1, *a2) {
                                        return Ok(v);
                                    }
                                }
                                FnType::InternalFn(ref f) => {
                                    if f.params.len() != 2 {
                                        return Err(EvalAltResult::ErrorFunctionArgMismatch);
                                    }

                                    let mut new_scope: Scope = Scope::new();
                                    let result1 = self.call_fn("clone",
                                                               Some(a1),
                                                               None,
                                                               None,
                                                               None,
                                                               None,
                                                               None);
                                    let result2 = self.call_fn("clone",
                                                               Some(a2),
                                                               None,
                                                               None,
                                                               None,
                                                               None,
                                                               None);
                                    match (result1, result2) {
                                        (Ok(r1), Ok(r2)) => {
                                            new_scope.push((f.params[0].clone(), r1));
                                            new_scope.push((f.params[1].clone(), r2));
                                        }
                                        _ => return Err(EvalAltResult::ErrorFunctionArgMismatch),
                                    }
                                    match self.eval_stmt(&mut new_scope, &*f.body) {
                                        Err(EvalAltResult::Return(x)) => return Ok(x),
                                        x => return x,
                                    }
                                }
                                _ => (),
                            }
                        }
                        Err(EvalAltResult::ErrorFunctionArgMismatch)
                    }
                    (Some(ref mut a1), None, None, None, None, None) => {
                        for arr_f in vf {
                            match *arr_f {
                                FnType::ExternalFn1(ref f) => {
                                    if let Ok(v) = f(*a1) {
                                        return Ok(v);
                                    }
                                }
                                FnType::InternalFn(ref f) => {
                                    if f.params.len() != 1 {
                                        return Err(EvalAltResult::ErrorFunctionArgMismatch);
                                    }

                                    let mut new_scope: Scope = Scope::new();
                                    let result1 = self.call_fn("clone",
                                                               Some(a1),
                                                               None,
                                                               None,
                                                               None,
                                                               None,
                                                               None);
                                    match result1 {
                                        Ok(r1) => {
                                            new_scope.push((f.params[0].clone(), r1));
                                        }
                                        _ => return Err(EvalAltResult::ErrorFunctionArgMismatch),
                                    }
                                    match self.eval_stmt(&mut new_scope, &*f.body) {
                                        Err(EvalAltResult::Return(x)) => return Ok(x),
                                        x => return x,
                                    }
                                }
                                _ => (),
                            }
                        }
                        Err(EvalAltResult::ErrorFunctionArgMismatch)
                    }
                    _ => {
                        for arr_f in vf {
                            match *arr_f {
                                FnType::ExternalFn0(ref f) => {
                                    if let Ok(v) = f() {
                                        return Ok(v);
                                    }
                                }
                                FnType::InternalFn(ref f) => {
                                    if !f.params.is_empty() {
                                        return Err(EvalAltResult::ErrorFunctionArgMismatch);
                                    }

                                    let mut new_scope: Scope = Scope::new();
                                    match self.eval_stmt(&mut new_scope, &*f.body) {
                                        Err(EvalAltResult::Return(x)) => return Ok(x),
                                        x => return x,
                                    }
                                }
                                _ => (),
                            }
                        }
                        Err(EvalAltResult::ErrorFunctionArgMismatch)
                    }
                }
            }
            None => Err(EvalAltResult::ErrorFunctionNotFound),
        }
    }

    /// Register a type for use with Engine. Keep in mind that
    /// your type must implement Clone.
    pub fn register_type<T: Clone + Any>(&mut self) {
        fn clone_helper<T: Clone>(t: T) -> T {
            t.clone()
        };

        self.register_fn("clone", clone_helper as fn(T) -> T);
    }

    /// Register a get function for a member of a registered type
    pub fn register_get<T: Clone + Any, U: Clone + Any, F>(&mut self, name: &str, get_fn: F)
        where F: 'static + Fn(&mut T) -> U
    {

        let get_name = "get$".to_string() + name;
        self.register_fn(&get_name, get_fn);
    }

    /// Register a set function for a member of a registered type
    pub fn register_set<T: Clone + Any, U: Clone + Any, F>(&mut self, name: &str, set_fn: F)
        where F: 'static + Fn(&mut T, U) -> ()
    {

        let set_name = "set$".to_string() + name;
        self.register_fn(&set_name, set_fn);
    }

    /// Shorthand for registering both getters and setters
    pub fn register_get_set<T: Clone + Any, U: Clone + Any, F, G>(&mut self,
                                                                  name: &str,
                                                                  get_fn: F,
                                                                  set_fn: G)
        where F: 'static + Fn(&mut T) -> U,
              G: 'static + Fn(&mut T, U) -> ()
    {

        self.register_get(name, get_fn);
        self.register_set(name, set_fn);
    }

    fn get_dot_val_helper(&self,
                          scope: &mut Scope,
                          this_ptr: &mut Box<Any>,
                          dot_rhs: &Expr)
                          -> Result<Box<Any>, EvalAltResult> {
        match *dot_rhs {
            Expr::FnCall(ref fn_name, ref args) => {
                if args.is_empty() {
                    self.call_fn(fn_name, Some(this_ptr), None, None, None, None, None)
                } else if args.len() == 1 {
                    let mut arg = self.eval_expr(scope, &args[0])?;

                    self.call_fn(fn_name,
                                 Some(this_ptr),
                                 Some(&mut arg),
                                 None,
                                 None,
                                 None,
                                 None)
                } else if args.len() == 2 {
                    let mut arg1 = self.eval_expr(scope, &args[0])?;
                    let mut arg2 = self.eval_expr(scope, &args[1])?;

                    self.call_fn(fn_name,
                                 Some(this_ptr),
                                 Some(&mut arg1),
                                 Some(&mut arg2),
                                 None,
                                 None,
                                 None)
                } else if args.len() == 3 {
                    let mut arg1 = self.eval_expr(scope, &args[0])?;
                    let mut arg2 = self.eval_expr(scope, &args[1])?;
                    let mut arg3 = self.eval_expr(scope, &args[2])?;

                    self.call_fn(fn_name,
                                 Some(this_ptr),
                                 Some(&mut arg1),
                                 Some(&mut arg2),
                                 Some(&mut arg3),
                                 None,
                                 None)
                } else if args.len() == 4 {
                    let mut arg1 = self.eval_expr(scope, &args[0])?;
                    let mut arg2 = self.eval_expr(scope, &args[1])?;
                    let mut arg3 = self.eval_expr(scope, &args[2])?;
                    let mut arg4 = self.eval_expr(scope, &args[3])?;

                    self.call_fn(fn_name,
                                 Some(this_ptr),
                                 Some(&mut arg1),
                                 Some(&mut arg2),
                                 Some(&mut arg3),
                                 Some(&mut arg4),
                                 None)
                } else if args.len() == 5 {
                    let mut arg1 = self.eval_expr(scope, &args[0])?;
                    let mut arg2 = self.eval_expr(scope, &args[1])?;
                    let mut arg3 = self.eval_expr(scope, &args[2])?;
                    let mut arg4 = self.eval_expr(scope, &args[3])?;
                    let mut arg5 = self.eval_expr(scope, &args[4])?;

                    self.call_fn(fn_name,
                                 Some(this_ptr),
                                 Some(&mut arg1),
                                 Some(&mut arg2),
                                 Some(&mut arg3),
                                 Some(&mut arg4),
                                 Some(&mut arg5))
                } else {
                    Err(EvalAltResult::ErrorFunctionCallNotSupported)
                }
            }
            Expr::Identifier(ref id) => {
                let get_fn_name = "get$".to_string() + id;
                self.call_fn(&get_fn_name, Some(this_ptr), None, None, None, None, None)
            }
            Expr::Index(ref id, ref idx_raw) => {
                let idx = self.eval_expr(scope, idx_raw)?;

                let get_fn_name = "get$".to_string() + id;

                if let Ok(mut val) = self.call_fn(&get_fn_name,
                                                  Some(this_ptr),
                                                  None,
                                                  None,
                                                  None,
                                                  None,
                                                  None) {
                    if let Ok(i) = idx.downcast::<i64>() {
                        if let Some(arr_typed) =
                               (*val).downcast_mut() as Option<&mut Vec<Box<Any>>> {
                            return self.call_fn("clone",
                                                Some(&mut arr_typed[*i as usize]),
                                                None,
                                                None,
                                                None,
                                                None,
                                                None);
                        } else {
                            return Err(EvalAltResult::ErrorIndexMismatch);
                        }
                    } else {
                        return Err(EvalAltResult::ErrorIndexMismatch);
                    }
                } else {
                    return Err(EvalAltResult::ErrorIndexMismatch);
                }
            }
            Expr::Dot(ref inner_lhs, ref inner_rhs) => {
                match **inner_lhs {
                    Expr::Identifier(ref id) => {
                        let get_fn_name = "get$".to_string() + id;
                        let result = self.call_fn(&get_fn_name,
                                                  Some(this_ptr),
                                                  None,
                                                  None,
                                                  None,
                                                  None,
                                                  None);

                        match result {
                            Ok(mut v) => self.get_dot_val_helper(scope, &mut v, inner_rhs),
                            e => e,
                        }
                    }
                    _ => Err(EvalAltResult::InternalErrorMalformedDotExpression),
                }
            }
            _ => Err(EvalAltResult::InternalErrorMalformedDotExpression),
        }
    }

    fn get_dot_val(&self,
                   scope: &mut Scope,
                   dot_lhs: &Expr,
                   dot_rhs: &Expr)
                   -> Result<Box<Any>, EvalAltResult> {
        match *dot_lhs {
            Expr::Identifier(ref id) => {
                let mut target: Option<Box<Any>> = None;

                for &mut (ref name, ref mut val) in &mut scope.iter_mut().rev() {
                    if *id == *name {
                        let result = self.call_fn("clone", Some(val), None, None, None, None, None);

                        if let Ok(clone) = result {
                            target = Some(clone);
                            break;
                        } else {
                            return result;
                        }
                    }
                }

                if let Some(mut t) = target {
                    let result = self.get_dot_val_helper(scope, &mut t, dot_rhs);

                    for &mut (ref name, ref mut val) in &mut scope.iter_mut().rev() {
                        if *id == *name {
                            *val = t;
                            break;
                        }
                    }
                    return result;
                }

                Err(EvalAltResult::ErrorVariableNotFound(id.clone()))
            }
            Expr::Index(ref id, ref idx_raw) => {
                let idx_boxed = self.eval_expr(scope, idx_raw)?;
                let idx = if let Ok(i) = idx_boxed.downcast::<i64>() {
                    i
                } else {
                    return Err(EvalAltResult::ErrorIndexMismatch);
                };

                let mut target: Option<Box<Any>> = None;

                for &mut (ref name, ref mut val) in &mut scope.iter_mut().rev() {
                    if *id == *name {
                        if let Some(arr_typed) =
                               (*val).downcast_mut() as Option<&mut Vec<Box<Any>>> {
                            let result = self.call_fn("clone",
                                                      Some(&mut arr_typed[*idx as usize]),
                                                      None,
                                                      None,
                                                      None,
                                                      None,
                                                      None);

                            if let Ok(clone) = result {
                                target = Some(clone);
                                break;
                            } else {
                                return result;
                            }
                        } else {
                            return Err(EvalAltResult::ErrorIndexMismatch);
                        }
                    }
                }

                if let Some(mut t) = target {
                    let result = self.get_dot_val_helper(scope, &mut t, dot_rhs);
                    for &mut (ref name, ref mut val) in &mut scope.iter_mut().rev() {
                        if *id == *name {
                            if let Some(arr_typed) =
                                   (*val).downcast_mut() as Option<&mut Vec<Box<Any>>> {
                                arr_typed[*idx as usize] = t;
                                break;
                            }
                        }
                    }
                    return result;
                }
                Err(EvalAltResult::ErrorVariableNotFound(id.clone()))
            }
            _ => Err(EvalAltResult::InternalErrorMalformedDotExpression),
        }
    }

    fn set_dot_val_helper(&self,
                          this_ptr: &mut Box<Any>,
                          dot_rhs: &Expr,
                          mut source_val: Box<Any>)
                          -> Result<Box<Any>, EvalAltResult> {
        match *dot_rhs {
            Expr::Identifier(ref id) => {
                let set_fn_name = "set$".to_string() + id;
                self.call_fn(&set_fn_name,
                             Some(this_ptr),
                             Some(&mut source_val),
                             None,
                             None,
                             None,
                             None)
            }
            Expr::Dot(ref inner_lhs, ref inner_rhs) => {
                match **inner_lhs {
                    Expr::Identifier(ref id) => {
                        let get_fn_name = "get$".to_string() + id;
                        let result = self.call_fn(&get_fn_name,
                                                  Some(this_ptr),
                                                  None,
                                                  None,
                                                  None,
                                                  None,
                                                  None);

                        match result {
                            Ok(mut v) => {
                                match self.set_dot_val_helper(&mut v, inner_rhs, source_val) {
                                    Ok(_) => {
                                        let set_fn_name = "set$".to_string() + id;

                                        self.call_fn(&set_fn_name,
                                                     Some(this_ptr),
                                                     Some(&mut v),
                                                     None,
                                                     None,
                                                     None,
                                                     None)
                                    }
                                    e => e,
                                }
                            }
                            e => e,
                        }

                    }
                    _ => Err(EvalAltResult::InternalErrorMalformedDotExpression),
                }
            }
            _ => Err(EvalAltResult::InternalErrorMalformedDotExpression),
        }
    }

    fn set_dot_val(&self,
                   scope: &mut Scope,
                   dot_lhs: &Expr,
                   dot_rhs: &Expr,
                   source_val: Box<Any>)
                   -> Result<Box<Any>, EvalAltResult> {
        match *dot_lhs {
            Expr::Identifier(ref id) => {
                let mut target: Option<Box<Any>> = None;

                for &mut (ref name, ref mut val) in &mut scope.iter_mut().rev() {
                    if *id == *name {
                        if let Ok(clone) = self.call_fn("clone",
                                                        Some(val),
                                                        None,
                                                        None,
                                                        None,
                                                        None,
                                                        None) {
                            target = Some(clone);
                            break;
                        } else {
                            return Err(EvalAltResult::ErrorVariableNotFound(id.clone()));
                        }
                    }
                }

                if let Some(mut t) = target {
                    let result = self.set_dot_val_helper(&mut t, dot_rhs, source_val);

                    for &mut (ref name, ref mut val) in &mut scope.iter_mut().rev() {
                        if *id == *name {
                            *val = t;
                            break;
                        }
                    }
                    return result;
                }

                Err(EvalAltResult::ErrorAssignmentToUnknownLHS)
            }
            Expr::Index(ref id, ref idx_raw) => {
                let idx_boxed = self.eval_expr(scope, idx_raw)?;
                let idx = if let Ok(i) = idx_boxed.downcast::<i64>() {
                    i
                } else {
                    return Err(EvalAltResult::ErrorIndexMismatch);
                };

                let mut target: Option<Box<Any>> = None;

                for &mut (ref name, ref mut val) in &mut scope.iter_mut().rev() {
                    if *id == *name {
                        if let Some(arr_typed) =
                               (*val).downcast_mut() as Option<&mut Vec<Box<Any>>> {
                            let result = self.call_fn("clone",
                                                      Some(&mut arr_typed[*idx as usize]),
                                                      None,
                                                      None,
                                                      None,
                                                      None,
                                                      None);

                            if let Ok(clone) = result {
                                target = Some(clone);
                                break;
                            } else {
                                return result;
                            }
                        } else {
                            return Err(EvalAltResult::ErrorIndexMismatch);
                        }
                    }
                }

                if let Some(mut t) = target {
                    let result = self.set_dot_val_helper(&mut t, dot_rhs, source_val);
                    for &mut (ref name, ref mut val) in &mut scope.iter_mut().rev() {
                        if *id == *name {
                            if let Some(arr_typed) =
                                   (*val).downcast_mut() as Option<&mut Vec<Box<Any>>> {
                                arr_typed[*idx as usize] = t;
                                break;
                            }
                        }
                    }
                    return result;
                }

                Err(EvalAltResult::ErrorVariableNotFound(id.clone()))
            }
            _ => Err(EvalAltResult::InternalErrorMalformedDotExpression),
        }
    }

    fn eval_expr(&self, scope: &mut Scope, expr: &Expr) -> Result<Box<Any>, EvalAltResult> {
        match *expr {
            Expr::IntConst(i) => Ok(Box::new(i)),
            Expr::FloatConst(i) => Ok(Box::new(i)),
            Expr::StringConst(ref s) => Ok(Box::new(s.clone())),
            Expr::CharConst(ref c) => Ok(Box::new(*c)),
            Expr::Identifier(ref id) => {
                for &mut (ref name, ref mut val) in &mut scope.iter_mut().rev() {
                    if *id == *name {
                        return self.call_fn("clone", Some(val), None, None, None, None, None);
                    }
                }

                #[cfg(feature = "modules")]
                {
                    if let Some(&(ref mod_name, ref _symbol, ref use_type)) =
                        scope
                        .uses
                        .iter()
                        .find(|x| x.1 == *id)
                    {
                        if *use_type != UseType::Symbol { return Err(EvalAltResult::ErrorVariableNotFound(id.clone())) }
                        let module = if let Some(m) = scope.symbols.iter().find(|x| x.0 == *mod_name) {
                            match m.1.downcast_ref::<Module>() {
                                Some(md) => md,
                                None => return Err(EvalAltResult::ErrorVariableNotFound(id.clone())),
                            }
                        } else { return Err(EvalAltResult::ErrorVariableNotFound(id.clone())) };
                        for &mut (ref name, ref mut val) in &mut module.scope.lock().unwrap().iter_mut().rev() {
                            if *id == *name {
                                return self.call_fn("clone", Some(val), None, None, None, None, None);
                            }
                        }
                    }
                }

                Err(EvalAltResult::ErrorVariableNotFound(id.clone()))
            }
            Expr::Index(ref id, ref idx_raw) => {
                let idx = self.eval_expr(scope, idx_raw)?;

                for &mut (ref name, ref mut val) in &mut scope.iter_mut().rev() {
                    if *id == *name {
                        if let Ok(i) = idx.downcast::<i64>() {
                            if let Some(arr_typed) =
                                   (*val).downcast_mut() as Option<&mut Vec<Box<Any>>> {
                                return self.call_fn("clone",
                                                    Some(&mut arr_typed[*i as usize]),
                                                    None,
                                                    None,
                                                    None,
                                                    None,
                                                    None);
                            } else {
                                return Err(EvalAltResult::ErrorIndexMismatch);
                            }
                        } else {
                            return Err(EvalAltResult::ErrorIndexMismatch);
                        }
                    }
                }

                Err(EvalAltResult::ErrorVariableNotFound(id.clone()))
            }
            Expr::Assignment(ref id, ref rhs) => {
                let rhs_val = self.eval_expr(scope, rhs)?;

                match **id {
                    Expr::Identifier(ref n) => {
                        for &mut (ref name, ref mut val) in &mut scope.iter_mut().rev() {
                            if *n == *name {

                                *val = rhs_val;

                                return Ok(Box::new(()));
                            }
                        }
                        Err(EvalAltResult::ErrorVariableNotFound(n.clone()))
                    }
                    Expr::Index(ref id, ref idx_raw) => {
                        let idx = self.eval_expr(scope, idx_raw)?;

                        for &mut (ref name, ref mut val) in &mut scope.iter_mut().rev() {
                            if *id == *name {
                                if let Ok(i) = idx.downcast::<i64>() {
                                    if let Some(arr_typed) =
                                           (*val).downcast_mut() as Option<&mut Vec<Box<Any>>> {
                                        arr_typed[*i as usize] = rhs_val;
                                        return Ok(Box::new(()));
                                    } else {
                                        return Err(EvalAltResult::ErrorIndexMismatch);
                                    }
                                } else {
                                    return Err(EvalAltResult::ErrorIndexMismatch);
                                }
                            }
                        }

                        Err(EvalAltResult::ErrorVariableNotFound(id.clone()))
                    }
                    Expr::Dot(ref dot_lhs, ref dot_rhs) => {
                        self.set_dot_val(scope, dot_lhs, dot_rhs, rhs_val)
                    }
                    _ => Err(EvalAltResult::ErrorAssignmentToUnknownLHS),
                }
            }
            Expr::Dot(ref lhs, ref rhs) => self.get_dot_val(scope, lhs, rhs),
            Expr::Array(ref contents) => {
                let mut arr = Vec::new();

                for item in &(*contents) {
                    let arg = self.eval_expr(scope, item)?;
                    arr.push(arg);
                }

                Ok(Box::new(arr))
            }
            Expr::FnCall(ref fn_name, ref args) => {
                if args.is_empty() {
                    #[cfg(feature = "modules")]
                    {
                        // check if fn exists
                        if self.fns.iter().any(|x| *x.0 == *fn_name) {
                           self.call_fn(fn_name, None, None, None, None, None, None)
                        } else if let Some(&(ref module, ..)) = scope.uses.iter().find(|x| x.1 == *fn_name && x.2 == UseType::Function) {
                            if let Some(&(.., ref md)) = scope.symbols.iter().find(|x| *x.0 == *module) {
                                match md.downcast_ref::<Module>() {
                                    Some(modul) => modul.engine.call_fn(fn_name,
                                                                        None,
                                                                        None,
                                                                        None,
                                                                        None,
                                                                        None,
                                                                        None),
                                    None => Err(EvalAltResult::ErrorNotAModule),
                                }
                            } else { Err(EvalAltResult::ErrorModuleNotFound) }
                        } else {
                            Err(EvalAltResult::ErrorFunctionNotFound)
                        }
                    }
                    #[cfg(not(feature = "modules"))]
                    {
                        self.call_fn(fn_name, None, None, None, None, None, None)
                    }
                } else if args.len() == 1 {
                    let mut arg = self.eval_expr(scope, &args[0])?;

                    #[cfg(feature = "modules")]
                    {
                        if self.fns.iter().any(|x| *x.0 == *fn_name) {
                           self.call_fn(fn_name, Some(&mut arg), None, None, None, None, None)
                        } else if let Some(&(ref module, ..)) = scope.uses.iter().find(|x| x.1 == *fn_name && x.2 == UseType::Function) {
                            if let Some(&(.., ref md)) = scope.symbols.iter().find(|x| *x.0 == *module) {
                                match md.downcast_ref::<Module>() {
                                    Some(modul) => modul.engine.call_fn(fn_name,
                                                                        Some(&mut arg),
                                                                        None,
                                                                        None,
                                                                        None,
                                                                        None,
                                                                        None),
                                    None => Err(EvalAltResult::ErrorNotAModule),
                                }
                            } else { Err(EvalAltResult::ErrorModuleNotFound) }
                        } else {
                            Err(EvalAltResult::ErrorFunctionNotFound)
                        }
                    }
                    #[cfg(not(feature = "modules"))]
                    {
                        self.call_fn(fn_name,
                                         Some(&mut arg),
                                         None,
                                         None,
                                         None,
                                         None,
                                         None)
                    }
                } else if args.len() == 2 {
                    let mut arg1 = self.eval_expr(scope, &args[0])?;
                    let mut arg2 = self.eval_expr(scope, &args[1])?;

                    #[cfg(feature = "modules")]
                    {
                        if self.fns.iter().any(|x| *x.0 == *fn_name) {
                            self.call_fn(fn_name,
                                         Some(&mut arg1),
                                         Some(&mut arg2),
                                         None,
                                         None,
                                         None,
                                         None)
                        } else if let Some(&(ref module, ..)) = scope.uses.iter().find(|x| x.1 == *fn_name && x.2 == UseType::Function) {
                            if let Some(&(.., ref md)) = scope.symbols.iter().find(|x| *x.0 == *module) {
                                match md.downcast_ref::<Module>() {
                                    Some(modul) => modul.engine.call_fn(fn_name,
                                                                        Some(&mut arg1),
                                                                        Some(&mut arg2),
                                                                        None,
                                                                        None,
                                                                        None,
                                                                        None),
                                    None => Err(EvalAltResult::ErrorNotAModule),
                                }
                            } else { Err(EvalAltResult::ErrorModuleNotFound) }
                        } else {
                            Err(EvalAltResult::ErrorFunctionNotFound)
                        }
                    }
                    #[cfg(not(feature = "modules"))]
                    {
                        self.call_fn(fn_name,
                                         Some(&mut arg1),
                                         Some(&mut arg2),
                                         None,
                                         None,
                                         None,
                                         None)
                    }
                } else if args.len() == 3 {
                    let mut arg1 = self.eval_expr(scope, &args[0])?;
                    let mut arg2 = self.eval_expr(scope, &args[1])?;
                    let mut arg3 = self.eval_expr(scope, &args[2])?;

                    #[cfg(feature = "modules")]
                    {
                        if self.fns.iter().any(|x| *x.0 == *fn_name) {
                            self.call_fn(fn_name,
                                         Some(&mut arg1),
                                         Some(&mut arg2),
                                         Some(&mut arg3),
                                         None,
                                         None,
                                         None)
                        } else if let Some(&(ref module, ..)) = scope.uses.iter().find(|x| x.1 == *fn_name && x.2 == UseType::Function) {
                            if let Some(&(.., ref md)) = scope.symbols.iter().find(|x| *x.0 == *module) {
                                match md.downcast_ref::<Module>() {
                                    Some(modul) => modul.engine.call_fn(fn_name,
                                                                        Some(&mut arg1),
                                                                        Some(&mut arg2),
                                                                        Some(&mut arg3),
                                                                        None,
                                                                        None,
                                                                        None),
                                    None => Err(EvalAltResult::ErrorNotAModule),
                                }
                            } else { Err(EvalAltResult::ErrorModuleNotFound) }
                        } else { Err(EvalAltResult::ErrorFunctionNotFound) }
                    }
                    #[cfg(not(feature = "modules"))]
                    {
                        self.call_fn(fn_name,
                                         Some(&mut arg1),
                                         Some(&mut arg2),
                                         Some(&mut arg3),
                                         None,
                                         None,
                                         None)
                    }
                } else if args.len() == 4 {
                    let mut arg1 = self.eval_expr(scope, &args[0])?;
                    let mut arg2 = self.eval_expr(scope, &args[1])?;
                    let mut arg3 = self.eval_expr(scope, &args[2])?;
                    let mut arg4 = self.eval_expr(scope, &args[3])?;

                    #[cfg(feature = "modules")]
                    {
                        if self.fns.iter().any(|x| *x.0 == *fn_name) {
                            self.call_fn(fn_name,
                                         Some(&mut arg1),
                                         Some(&mut arg2),
                                         Some(&mut arg3),
                                         Some(&mut arg4),
                                         None,
                                         None)
                        } else if let Some(&(ref module, ..)) = scope.uses.iter().find(|x| x.1 == *fn_name && x.2 == UseType::Function) {
                            if let Some(&(.., ref md)) = scope.symbols.iter().find(|x| *x.0 == *module) {
                                match md.downcast_ref::<Module>() {
                                    Some(modul) => modul.engine.call_fn(fn_name,
                                                                        Some(&mut arg1),
                                                                        Some(&mut arg2),
                                                                        Some(&mut arg3),
                                                                        Some(&mut arg4),
                                                                        None,
                                                                        None),
                                    None => Err(EvalAltResult::ErrorNotAModule),
                                }
                            } else { Err(EvalAltResult::ErrorModuleNotFound) }
                        } else {
                            Err(EvalAltResult::ErrorFunctionNotFound)
                        }
                    }
                    #[cfg(not(feature = "modules"))]
                    {
                        self.call_fn(fn_name,
                                         Some(&mut arg1),
                                         Some(&mut arg2),
                                         Some(&mut arg3),
                                         Some(&mut arg4),
                                         None,
                                         None)
                    }
                } else if args.len() == 5 {
                    let mut arg1 = self.eval_expr(scope, &args[0])?;
                    let mut arg2 = self.eval_expr(scope, &args[1])?;
                    let mut arg3 = self.eval_expr(scope, &args[2])?;
                    let mut arg4 = self.eval_expr(scope, &args[3])?;
                    let mut arg5 = self.eval_expr(scope, &args[4])?;

                    #[cfg(feature = "modules")]
                    {
                        if self.fns.iter().any(|x| *x.0 == *fn_name) {
                            self.call_fn(fn_name,
                                         Some(&mut arg1),
                                         Some(&mut arg2),
                                         Some(&mut arg3),
                                         Some(&mut arg4),
                                         Some(&mut arg5),
                                         None)
                        } else if let Some(&(ref module, ..)) = scope.uses.iter().find(|x| x.1 == *fn_name && x.2 == UseType::Function) {
                                if let Some(&(.., ref md)) = scope.symbols.iter().find(|x| *x.0 == *module) {
                                    match md.downcast_ref::<Module>() {
                                        Some(modul) => modul.engine.call_fn(fn_name,
                                                                            Some(&mut arg1),
                                                                            Some(&mut arg2),
                                                                            Some(&mut arg3),
                                                                            Some(&mut arg4),
                                                                            Some(&mut arg5),
                                                                            None),
                                        None => Err(EvalAltResult::ErrorNotAModule),
                                    }
                                } else { Err(EvalAltResult::ErrorModuleNotFound) }
                        } else {
                            Err(EvalAltResult::ErrorFunctionNotFound)
                        }
                    }
                    #[cfg(not(feature = "modules"))]
                    {
                        self.call_fn(fn_name,
                                         Some(&mut arg1),
                                         Some(&mut arg2),
                                         Some(&mut arg3),
                                         Some(&mut arg4),
                                         Some(&mut arg5),
                                         None)
                    }
                } else if args.len() == 6 {
                    let mut arg1 = self.eval_expr(scope, &args[0])?;
                    let mut arg2 = self.eval_expr(scope, &args[1])?;
                    let mut arg3 = self.eval_expr(scope, &args[2])?;
                    let mut arg4 = self.eval_expr(scope, &args[3])?;
                    let mut arg5 = self.eval_expr(scope, &args[4])?;
                    let mut arg6 = self.eval_expr(scope, &args[5])?;

                    #[cfg(feature = "modules")]
                    {
                        if self.fns.iter().any(|x| *x.0 == *fn_name) {
                            self.call_fn(fn_name,
                                         Some(&mut arg1),
                                         Some(&mut arg2),
                                         Some(&mut arg3),
                                         Some(&mut arg4),
                                         Some(&mut arg5),
                                         Some(&mut arg6))
                        } else if let Some(&(ref module, ..)) = scope.uses.iter().find(|x| x.1 == *fn_name && x.2 == UseType::Function) {
                                if let Some(&(.., ref md)) = scope.symbols.iter().find(|x| *x.0 == *module) {
                                    match md.downcast_ref::<Module>() {
                                        Some(modul) => modul.engine.call_fn(fn_name,
                                                                            Some(&mut arg1),
                                                                            Some(&mut arg2),
                                                                            Some(&mut arg3),
                                                                            Some(&mut arg4),
                                                                            Some(&mut arg5),
                                                                            Some(&mut arg6)),
                                        None => Err(EvalAltResult::ErrorNotAModule),
                                    }
                                } else { Err(EvalAltResult::ErrorModuleNotFound) }
                        } else {
                            Err(EvalAltResult::ErrorFunctionNotFound)
                        }
                    }
                    #[cfg(not(feature = "modules"))]
                    {
                        self.call_fn(fn_name,
                                         Some(&mut arg1),
                                         Some(&mut arg2),
                                         Some(&mut arg3),
                                         Some(&mut arg4),
                                         Some(&mut arg5),
                                         Some(&mut arg6))
                    }
                } else {
                    Err(EvalAltResult::ErrorFunctionCallNotSupported)
                }
            }
            #[cfg(feature = "modules")]
            Expr::Import(ref args) => {
                let mut arg_res = match self.eval_expr(scope, &args[0]) {
                    Ok(a) => a,
                    Err(e) => return Err(e),
                };

                let mut arg = match arg_res.downcast::<String>() {
                    Ok(a) => a,
                    Err(_) => return Err(EvalAltResult::ErrorFunctionArgMismatch),
                };

                match Module::import(arg.as_ref()) {
                    Ok(mut m) => Ok(Box::new({m.exec(self); m})),
                    Err(e) => Err(EvalAltResult::ModuleError(e)),
                }
            }
            Expr::True => Ok(Box::new(true)),
            Expr::False => Ok(Box::new(false)),
        }
    }

    fn eval_stmt(&self, scope: &mut Scope, stmt: &Stmt) -> Result<Box<Any>, EvalAltResult> {
        match *stmt {
            Stmt::Expr(ref e) => self.eval_expr(scope, e),
            Stmt::Block(ref b) => {
                let prev_len = scope.len();
                let mut last_result: Result<Box<Any>, EvalAltResult> = Ok(Box::new(()));

                for s in b.iter() {
                    last_result = self.eval_stmt(scope, s);
                    if let Err(x) = last_result {
                        last_result = Err(x);
                        break;
                    }
                }

                while scope.len() > prev_len {
                    scope.pop();
                }

                last_result
            }
            Stmt::If(ref guard, ref body) => {
                let guard_result = self.eval_expr(scope, guard)?;
                match guard_result.downcast::<bool>() {
                    Ok(g) => {
                        if *g {
                            self.eval_stmt(scope, body)
                        } else {
                            Ok(Box::new(()))
                        }
                    }
                    Err(_) => Err(EvalAltResult::ErrorIfGuardMismatch),
                }
            }
            Stmt::IfElse(ref guard, ref body, ref else_body) => {
                let guard_result = self.eval_expr(scope, guard)?;
                match guard_result.downcast::<bool>() {
                    Ok(g) => {
                        if *g {
                            self.eval_stmt(scope, body)
                        } else {
                            self.eval_stmt(scope, else_body)
                        }
                    }
                    Err(_) => Err(EvalAltResult::ErrorIfGuardMismatch),
                }
            }
            Stmt::While(ref guard, ref body) => {
                loop {
                    let guard_result = self.eval_expr(scope, guard)?;
                    match guard_result.downcast::<bool>() {
                        Ok(g) => {
                            if *g {
                                match self.eval_stmt(scope, body) {
                                    Err(EvalAltResult::LoopBreak) => {
                                        return Ok(Box::new(()));
                                    }
                                    Err(x) => {
                                        return Err(x);
                                    }
                                    _ => (),
                                }
                            } else {
                                return Ok(Box::new(()));
                            }
                        }
                        Err(_) => return Err(EvalAltResult::ErrorIfGuardMismatch),
                    }
                }
            }
            Stmt::Loop(ref body) => {
                loop {
                    match self.eval_stmt(scope, body) {
                        Err(EvalAltResult::LoopBreak) => {
                            return Ok(Box::new(()));
                        }
                        Err(x) => {
                            return Err(x);
                        }
                        _ => (),
                    }
                }
            }
            Stmt::Break => Err(EvalAltResult::LoopBreak),
            Stmt::Return => Err(EvalAltResult::Return(Box::new(()))),
            Stmt::ReturnWithVal(ref a) => {
                let result = self.eval_expr(scope, a)?;
                Err(EvalAltResult::Return(result))
            }
            Stmt::Var(ref name, ref init) => {
                match *init {
                    Some(ref v) => {
                        let i = self.eval_expr(scope, v)?;
                        scope.push((name.clone(), i));
                    }
                    None => {
                        scope.push((name.clone(), Box::new(())));
                    }
                };
                Ok(Box::new(()))
            }
            #[cfg(feature = "modules")]
            Stmt::Use(ref module, ref symbol) => {
                if let Some(&(_, ref symbol_any)) = scope.symbols.iter().find(|x| x.0 == *module) {
                    if let Some(rhai_module) = symbol_any.downcast_ref::<Module>() {
                        if rhai_module.is_erroneous { return Err(EvalAltResult::ErrorErroneousModule) }

                        if rhai_module.scope.lock().unwrap().symbols.iter().any(|x| x.0 == *symbol) {
                            scope.uses.push((module.clone(), symbol.clone(), UseType::Symbol));
                            return Ok(Box::new(()));
                        }
                        else if rhai_module.engine.fns.iter().any(|x| x.0 == symbol) {
                            scope.uses.push((module.clone(), symbol.clone(), UseType::Function));
                            return Ok(Box::new(()));
                        }

                        return Err(EvalAltResult::ErrorModuleMemberNotFound);
                    }
                    return Err(EvalAltResult::ErrorNotAModule);
                }

                Err(EvalAltResult::ErrorModuleNotFound)
            }
        }
    }

    /// Evaluate a file
    pub fn eval_file<T: Any + Clone>(&mut self, fname: &str) -> Result<T, EvalAltResult> {
        use std::fs::File;
        use std::io::prelude::*;

        if let Ok(mut f) = File::open(fname) {
            let mut contents = String::new();

            if f.read_to_string(&mut contents).is_ok() {
                self.eval::<T>(&contents)
            } else {
                Err(EvalAltResult::ErrorCantOpenScriptFile)
            }
        } else {
            Err(EvalAltResult::ErrorCantOpenScriptFile)
        }
    }

    /// Evaluate a string
    pub fn eval<T: Any + Clone>(&mut self, input: &str) -> Result<T, EvalAltResult> {
        let mut scope: Scope = Scope::new();

        self.eval_with_scope(&mut scope, input)
    }

    /// Evaluate with own scope
    pub fn eval_with_scope<T: Any + Clone>(&mut self,
                                           scope: &mut Scope,
                                           input: &str)
                                           -> Result<T, EvalAltResult> {
        let tokens = lex(input);

        let mut peekables = tokens.peekable();
        let tree = parse(&mut peekables);

        match tree {
            Ok((ref os, ref fns)) => {
                let mut x: Result<Box<Any>, EvalAltResult> = Ok(Box::new(()));

                for f in fns {
                    if f.params.len() > 6 {
                        return Err(EvalAltResult::ErrorFunctionArityNotSupported);
                    }
                    let name = f.name.clone();
                    let local_f = f.clone();
                    let ent = self.fns.entry(name).or_insert_with(Vec::new);
                    (*ent).push(FnType::InternalFn(local_f));
                }

                for o in os {
                    x = match self.eval_stmt(scope, o) {
                        Ok(v) => Ok(v),
                        Err(e) => return Err(e),
                    }
                }

                match x {
                    Ok(v) => {
                        match v.downcast::<T>() {
                            Ok(out) => Ok(*out),
                            Err(_) => Err(EvalAltResult::ErrorMismatchOutputType),
                        }
                    }
                    Err(e) => Err(e),
                }
            }
            Err(_) => Err(EvalAltResult::ErrorFunctionArgMismatch),
        }
    }

    /// Evaluate a file, but only return errors, if there are any.
    /// Useful for when you don't need the result, but still need
    /// to keep track of possible errors
    pub fn consume_file(&mut self, fname: &str) -> Result<(), EvalAltResult> {
        use std::fs::File;
        use std::io::prelude::*;

        if let Ok(mut f) = File::open(fname) {
            let mut contents = String::new();

            if f.read_to_string(&mut contents).is_ok() {
                if let e @ Err(_) = self.consume(&contents) {
                    return e;
                } else { return Ok(()); }
            } else {
                Err(EvalAltResult::ErrorCantOpenScriptFile)
            }
        } else {
            Err(EvalAltResult::ErrorCantOpenScriptFile)
        }
    }

    /// Evaluate a string, but only return errors, if there are any.
    /// Useful for when you don't need the result, but still need
    /// to keep track of possible errors
    pub fn consume(&mut self, input: &str) -> Result<(), EvalAltResult> {
        let mut scope: Scope = Scope::new();

        self.consume_with_scope(&mut scope, input)
    }

    /// Evaluate a string with own scoppe, but only return errors, if there are any.
    /// Useful for when you don't need the result, but still need
    /// to keep track of possible errors
    pub fn consume_with_scope(&mut self, scope: &mut Scope, input: &str) -> Result<(), EvalAltResult> {
        let tokens = lex(input);

        let mut peekables = tokens.peekable();
        let tree = parse(&mut peekables);

        match tree {
            Ok((ref os, ref fns)) => {
                for f in fns {
                    if f.params.len() > 6 {
                        return Ok(());
                    }
                    let name = f.name.clone();
                    let local_f = f.clone();
                    let ent = self.fns.entry(name).or_insert_with(Vec::new);
                    (*ent).push(FnType::InternalFn(local_f));
                }

                for o in os {
                    if let Err(e) = self.eval_stmt(scope, o) {
                        return Err(e);
                    }
                }

                Ok(())
            },
            Err(_) => Err(EvalAltResult::ErrorFunctionArgMismatch),
        }
    }

    /// Register the default library. That means, numberic types, char, bool
    /// String, arithmetics and string concatenations.
    pub fn register_default_lib(engine: &mut Engine) {
        engine.register_type::<i32>();
        engine.register_type::<u32>();
        engine.register_type::<i64>();
        engine.register_type::<u64>();
        engine.register_type::<f32>();
        engine.register_type::<f64>();
        engine.register_type::<String>();
        engine.register_type::<char>();
        engine.register_type::<bool>();

        #[cfg(module)]
        engine.register_type::<Module>();

        macro_rules! reg_op {
            ($engine:expr, $x:expr, $op:expr, $( $y:ty ),*) => (
                $(
                    $engine.register_fn($x, ($op as fn(x: $y, y: $y)->$y));
                )*
            )
        }

        macro_rules! reg_un {
            ($engine:expr, $x:expr, $op:expr, $( $y:ty ),*) => (
                $(
                    $engine.register_fn($x, ($op as fn(x: $y)->$y));
                )*
            )
        }

        macro_rules! reg_cmp {
            ($engine:expr, $x:expr, $op:expr, $( $y:ty ),*) => (
                $(
                    $engine.register_fn($x, ($op as fn(x: $y, y: $y)->bool));
                )*
            )
        }

        fn add<T: Add>(x: T, y: T) -> <T as Add>::Output { x + y }
        fn sub<T: Sub>(x: T, y: T) -> <T as Sub>::Output { x - y }
        fn mul<T: Mul>(x: T, y: T) -> <T as Mul>::Output { x * y }
        fn div<T: Div>(x: T, y: T) -> <T as Div>::Output { x / y }
        fn neg<T: Neg>(x: T) -> <T as Neg>::Output { -x }
        fn lt<T: PartialOrd>(x: T, y: T)  -> bool { x < y  }
        fn lte<T: PartialOrd>(x: T, y: T) -> bool { x <= y }
        fn gt<T: PartialOrd>(x: T, y: T)  -> bool { x > y  }
        fn gte<T: PartialOrd>(x: T, y: T) -> bool { x >= y }
        fn eq<T: PartialEq>(x: T, y: T)   -> bool { x == y }
        fn ne<T: PartialEq>(x: T, y: T)   -> bool { x != y }
        fn and(x: bool, y: bool)   -> bool { x && y }
        fn or(x: bool, y: bool)    -> bool { x || y }
        fn not(x: bool)            -> bool { !x }
        fn concat(x: String, y: String) -> String { x + &y }
        fn binary_and<T: BitAnd>(x: T, y: T)   -> <T as BitAnd>::Output { x & y }
        fn binary_or<T: BitOr>(x: T, y: T)   -> <T as BitOr>::Output { x | y }
        fn binary_xor<T: BitXor>(x: T, y: T)   -> <T as BitXor>::Output { x ^ y }
        fn left_shift<T: Shl<T>>(x: T, y: T)   -> <T as Shl<T>>::Output { x.shl(y) }
        fn right_shift<T: Shr<T>>(x: T, y: T)   -> <T as Shr<T>>::Output { x.shr(y) }
        fn modulo<T: Rem<T>>(x: T, y: T)   -> <T as Rem<T>>::Output { x % y}

        reg_op!(engine, "+", add, i32, i64, u32, u64, f32, f64);
        reg_op!(engine, "-", sub, i32, i64, u32, u64, f32, f64);
        reg_op!(engine, "*", mul, i32, i64, u32, u64, f32, f64);
        reg_op!(engine, "/", div, i32, i64, u32, u64, f32, f64);

        reg_cmp!(engine, "<", lt, i32, i64, u32, u64, String, f64);
        reg_cmp!(engine, "<=", lte, i32, i64, u32, u64, String, f64);
        reg_cmp!(engine, ">", gt, i32, i64, u32, u64, String, f64);
        reg_cmp!(engine, ">=", gte, i32, i64, u32, u64, String, f64);
        reg_cmp!(engine, "==", eq, i32, i64, u32, u64, bool, String, f64);
        reg_cmp!(engine, "!=", ne, i32, i64, u32, u64, bool, String, f64);

        reg_op!(engine, "||", or, bool);
        reg_op!(engine, "&&", and, bool);
        reg_op!(engine, "|", binary_or, i32, i64, u32, u64);
        reg_op!(engine, "|", or, bool);
        reg_op!(engine, "&", binary_and, i32, i64, u32, u64);
        reg_op!(engine, "&", and, bool);
        reg_op!(engine, "^", binary_xor, i32, i64, u32, u64);
        reg_op!(engine, "<<", left_shift, i32, i64, u32, u64);
        reg_op!(engine, ">>", right_shift, i32, i64, u32, u64);
        reg_op!(engine, "%", modulo, i32, i64, u32, u64);

        reg_un!(engine, "-", neg, i32, i64, f32, f64);
        reg_un!(engine, "!", not, bool);

        engine.register_fn("+", concat);

        // engine.register_fn("[]", idx);
        // FIXME?  Registering array lookups are a special case because we want to return boxes
        // directly let ent = engine.fns.entry("[]".to_string()).or_insert_with(Vec::new);
        // (*ent).push(FnType::ExternalFn2(Box::new(idx)));
    }

    pub fn module_fns(&mut self, register: fn(&mut Engine)) {
        self.module_register = Some(register);
    }

    /// Make a new engine
    pub fn new() -> Engine {
        let mut engine = Engine { fns: HashMap::new(), module_register: None };

        Engine::register_default_lib(&mut engine);

        engine
    }
}
