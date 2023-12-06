const Web3 = require('web3');
const fs = require('fs');

const DEPLOYER_PRIVATE_KEY = 'ac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80';

const main = async () => {
    if (process.argv.length < 3) {
        console.log(`You must specify path to the WASM binary!`);
        console.log(`Example: node deploy-contract.js --dev ./bin/greeting.wasm`);
        process.exit(-1);
    }
    let args = process.argv.slice(2);
    const checkFlag = (param) => {
        let indexOf = args.indexOf(param)
        if (indexOf < 0) {
            return false
        }
        args.splice(indexOf, 1)
        return true
    };
    let isLocal = checkFlag('--local')
    let isDev = checkFlag('--dev')

    let web3Url = 'https://rpc.dev0.fluentlabs.xyz/';
    if (isLocal) {
        web3Url = 'http://127.0.0.1:8545';
    }

    let [binaryPath] = args;
    let wasmBinary = fs.readFileSync(binaryPath).toString('hex');
    const web3 = new Web3(web3Url);
    let privateKey = process.env.DEPLOYER_PRIVATE_KEY || DEPLOYER_PRIVATE_KEY;

    console.log('Signing transaction...');
    const signedTransaction = await web3.eth.accounts.signTransaction({
        data: '0x' + wasmBinary,
        gas: 1_000_000,
    }, privateKey)

    console.log('Sending transaction...');
    const receipt = await web3.eth.sendSignedTransaction(signedTransaction.rawTransaction);
    console.log(`Receipt: ${JSON.stringify(receipt, null, 2)}`)

    const {contractAddress} = receipt;
    if (contractAddress) {
        console.log(`Contract address is: ${contractAddress}`);
    }

    // let contractAddress = '0xCf7Ed3AccA5a467e9e704C703E8D87F634fB0Fc9';
    const result = await web3.eth.call({
        to: contractAddress,
    });
    const message = web3.utils.hexToAscii(result)
    console.log(`Message: "${message}"`)

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