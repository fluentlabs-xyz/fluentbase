const {Web3, ETH_DATA_FORMAT} = require('web3');
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

    let web3Url = '';
    if (isLocal) {
        web3Url = 'http://127.0.0.1:8545';
    } else if (isDev) {
        web3Url = 'https://rpc.dev1.fluentlabs.xyz/';
    } else {
        console.log(`You must specify --dev or --local flag!`);
        console.log(`Example: node deploy-contract.js --dev ./bin/greeting.wasm`);
        process.exit(-1);
    }

    let [binaryPath] = args;
    let wasmBinary = fs.readFileSync(binaryPath).toString('hex');
    const web3 = new Web3(web3Url);
    let privateKey = process.env.DEPLOYER_PRIVATE_KEY || DEPLOYER_PRIVATE_KEY;
    let account = web3.eth.accounts.privateKeyToAccount('0x' + privateKey);

    console.log('Signing transaction...');
    const gasPrice = await web3.eth.getGasPrice(ETH_DATA_FORMAT)
    const signedTransaction = await web3.eth.accounts.signTransaction({
        data: '0x' + wasmBinary,
        gasPrice,
        gas: 10_000_000,
        from: account.address,
    }, privateKey)

    let contractAddress = '';
    console.log('Sending transaction...');
    await web3.eth.sendSignedTransaction(signedTransaction.rawTransaction)
       .on('confirmation', confirmation => {
           contractAddress = confirmation.receipt.contractAddress;
           console.log(confirmation)
           if (contractAddress) {
               console.log(`Contract address is: ${contractAddress}`);
           }
       });

    const result = await web3.eth.call({
        to: contractAddress,
    });
    function isASCII(str) {
        return /^[\x00-\x7F]*$/.test(str);
    }
    if (isASCII(web3.utils.hexToAscii(result))) {
        console.log(`Message: "${web3.utils.hexToAscii(result)}"`)
    } else {
        console.log(`Message: "${result}"`)
    }

    // const signedTransaction1 = await web3.eth.accounts.signTransaction({
    //     to: contractAddress,
    //     gas: 1_000_000,
    // }, DEPLOYER_PRIVATE_KEY)
    // const receipt1 = await web3.eth.sendSignedTransaction(signedTransaction1.rawTransaction);
    // console.log(`Receipt: ${JSON.stringify(receipt1, null, 2)}`)

    const latestMinedBlockNumber = await web3.eth.getBlockNumber();
    console.log(`Latest block number: ${latestMinedBlockNumber}`);

    process.exit(0)
}

main().then(console.log).catch(console.error);