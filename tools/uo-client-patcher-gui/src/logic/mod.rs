//! The patcher's logic, apart from the UI: the opened executable, the edit state and the patch it builds, the dry run,
//! the data check and the login server files.

pub mod edits;
pub mod login;
pub mod opened;
pub mod preview;

#[cfg(test)]
mod synthetic;
#[cfg(test)]
mod tests;
