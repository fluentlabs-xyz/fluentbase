# Client

Client is a macro that allows you to interact with a contract from the client side. It is a simple way to interact with a contract without having to write a lot of boilerplate code.

## Usage

```rs
#[client(mode = "solidity")]
pub trait RouterAPI {

    fn greeting(&mut self, message: String) -> String;
    fn custom_greeting(&mut self, message: String) -> String;
}
```


## Problems

1.
