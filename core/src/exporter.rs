pub trait Exporter {
    type Output;
    fn to_target(self) -> Self::Output;
}
