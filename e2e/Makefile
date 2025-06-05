FILES = assets/HelloWorld.bin assets/Storage.bin assets/CallWasm.bin assets/ERC20.bin assets/DelegateCaller.bin assets/Multicall.bin assets/Router.bin assets/ContractDeployer.bin assets/Blake2FCaller.bin assets/Caller.bin assets/EcrecoverWithLowGas.bin

assets/%.bin: assets/%.sol
	solc --bin --overwrite $< -o assets/

all: $(FILES)
