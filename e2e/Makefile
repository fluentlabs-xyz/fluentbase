FILES = assets/HelloWorld.bin assets/Storage.bin assets/CallWasm.bin assets/ERC20.bin

assets/%.bin: assets/%.sol
	solc --bin --overwrite $< -o assets/

all: $(FILES)