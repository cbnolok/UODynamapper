use bevy::app::Plugin;
use crate::logger;

pub fn log_plugin_build<T: TrackedPlugin>(plugin: &T) {
    let full_name = std::any::type_name::<T>();
    let bare_name = full_name.rsplit("::").next().unwrap();

    logger::one(
        None, //Some(false),
        logger::LogSev::Debug,
        logger::LogAbout::Plugins,
        &format!("Build: {bare_name} (registered by: {}).", plugin.registered_by()),
    );
}

fn log_system_add_base(myname: &'static str, plugname: &str, schedule: &'static str) {
    let plugname_bare = plugname.rsplit("::").next().unwrap();
    let myname_bare = myname.rsplit("::").next().unwrap();
    logger::one(
        None, //Some(false),
        logger::LogSev::Debug,
        logger::LogAbout::SystemsGeneral,
        &format!("Running with schedule {schedule}: {myname_bare} (registered by: {plugname_bare})."),
    );
}

pub fn log_system_add_startup<T: TrackedPlugin>(_myname: &'static str) {
    log_system_add_base(_myname, std::any::type_name::<T>(), "Startup")
}
pub fn log_system_add_update<T: TrackedPlugin>(_myname: &'static str) {
    () // do nothing for now, it can be too cluttering.
    //log_system_add_base(_myname, std::any::type_name::<T>(), "Update")
}

pub trait TrackedPlugin: Plugin {
    fn registered_by(&self) -> &str;
}

#[macro_export]
macro_rules! impl_tracked_plugin {
    ($plugin:ty) => {
        impl TrackedPlugin for $plugin {
            fn registered_by(&self) -> &str {
                self.registered_by
            }
        }
    };
}

