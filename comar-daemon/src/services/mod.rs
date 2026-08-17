use crate::error::Result;
use zvariant::OwnedValue;

mod network_stack;
mod package_handler;
mod user_manager;

pub enum ServiceDispatch {
    Native(Result<Vec<OwnedValue>>),
    Passthrough,
}

pub trait ServiceHandler: Send + Sync {
    fn handle(
        &self,
        method: &str,
        args: &[OwnedValue],
        lang: &str,
    ) -> Result<Vec<OwnedValue>>;
}

pub fn dispatch(model: &str, method: &str, args: &[OwnedValue], lang: &str) -> ServiceDispatch {
    match model {
        "Network.Stack" => {
            ServiceDispatch::Native(network_stack::NetworkStack.handle(method, args, lang))
        }
        "User.Manager" => {
            ServiceDispatch::Native(user_manager::UserManager.handle(method, args, lang))
        }
        "System.PackageHandler" => {
            ServiceDispatch::Native(package_handler::run_all(method, args))
        }
        _ => ServiceDispatch::Passthrough,
    }
}
