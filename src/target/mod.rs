pub mod text;

pub trait ZhangTarget<Target> {
    fn to_target(self) -> Target;
}
