use hara_protocol_macros::hara_protocol;

#[hara_protocol(namespace = "std.protocol.icomponent", name = "IComponent")]
pub trait IComponent {
    type Metadata;
    type QueryLevel;
    type Health;

    #[hara_method(value = "props", arity = 1)]
    fn props(&self) -> Self::Metadata;
    #[hara_method(value = "status", arity = 1)]
    fn status(&self) -> Self::Metadata;
    #[hara_method(value = "info", arity = 2)]
    fn info(&self, _level: &Self::QueryLevel) -> Self::Metadata {
        self.status()
    }
    #[hara_method(value = "health", arity = 1)]
    fn health(&self) -> Self::Health;
    #[hara_method(value = "started?", arity = 1)]
    fn started(&self) -> bool;
    #[hara_method(value = "stopped?", arity = 1)]
    fn stopped(&self) -> bool;
    #[hara_method(value = "start", arity = 1)]
    fn start(&mut self);
    #[hara_method(value = "stop", arity = 1)]
    fn stop(&mut self);
    #[hara_method(value = "kill", arity = 1)]
    fn kill(&mut self) {
        self.stop();
    }
    #[hara_method(value = "remote?", arity = 1)]
    fn remote(&self) -> bool {
        false
    }
}
