const Web3 = require('web3');
const fs = require('fs');

const DEPLOYER_PRIVATE_KEY = 'ac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80';

const main = async () => {
    let wasmBinary = fs.readFileSync('./bin/greeting.wasm').toString('hex');
    const web3 = new Web3('http://127.0.0.1:8545');

    console.log('Signing transaction...');
    const signedTransaction = await web3.eth.accounts.signTransaction({
        data: '0x' + wasmBinary,
        gas: 10_000_000,
    }, DEPLOYER_PRIVATE_KEY)
    console.log('Sending transaction...');
    const receipt = await web3.eth.sendSignedTransaction(signedTransaction.rawTransaction);
    console.log(`Receipt: ${JSON.stringify(receipt, null, 2)}`)
    const {contractAddress} = receipt;
    console.log(`Contract address is: ${contractAddress}`);

    // let contractAddress = '0xCf7Ed3AccA5a467e9e704C703E8D87F634fB0Fc9';
    // const result = await web3.eth.call({
    //     to: contractAddress,
    // });
    // const message = web3.utils.hexToAscii(result)
    // console.log(`Message: "${message}"`)

    // const signedTransaction1 = await web3.eth.accounts.signTransaction({
    //     to: contractAddress,
    //     gas: 10_000_000,
    // }, DEPLOYER_PRIVATE_KEY)
    // const receipt1 = await web3.eth.sendSignedTransaction(signedTransaction1.rawTransaction);
    // console.log(`Receipt: ${JSON.stringify(receipt1, null, 2)}`)

    const latestMinedBlockNumber = await web3.eth.getBlockNumber();
    console.log(`Latest block number: ${latestMinedBlockNumber}`);
}

main().then(console.log).catch(console.error);