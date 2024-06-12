use fluentbase_sdk::{
    codec::Encoder,
    types::{CoreInput, ICoreInput},
};

#[allow(dead_code)]
pub(crate) fn decode_method_input<T: Encoder<T> + Default>(input: &[u8]) -> T {
    let mut core_input = T::default();
    <CoreInput<T> as ICoreInput>::MethodData::decode_field_body(input, &mut core_input);
    core_input
}
