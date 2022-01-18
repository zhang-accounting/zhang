

pub mod text;
pub trait AvaroTarget<Target> {
    fn to_target(self) -> Target;
}
