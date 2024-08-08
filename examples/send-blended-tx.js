const {Web3} = require('web3');
const {hexToBytes, bytesToHex} = require('web3-utils');
const {ethRpcMethods} = require('web3-rpc-methods');

const DEPLOYER_PRIVATE_KEY = 'ac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80';

function dec2hex(n) {
    let res = n ? [n % 256].concat(dec2hex(~~(n / 256))) : [];
    return res.reverse()
}

const EMPTY_STRING_CODE = 0x80;
const EMPTY_LIST_CODE = 0xC0;

class RlpHeader {

    constructor(is_list = false, payload_length = 0) {
        this.is_list = is_list
        this.payload_length = payload_length
    }

    encode() {
        let out = []
        if (this.payload_length < 56) {
            let code = EMPTY_STRING_CODE
            if (this.is_list) {
                code = EMPTY_LIST_CODE
            }
            out.push(code + self.payload_length)
        } else {
            let len_be = dec2hex(this.payload_length);
            console.log(`len_be: ${JSON.stringify(len_be)}`)
            let code = 0xB7;
            if (this.is_list) {
                code = 0xF7
            }
            out.push(code + len_be.length)
            out.push(...len_be)
        }
        return out
    }
}

const main = async () => {
    if (process.argv.length < 2) {
        console.log(`You must specify local or remote flag`);
        console.log(`Example: node send-blended.js --local`);
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
        web3Url = 'https://rpc.dev.thefluent.xyz/';
    } else {
        console.log(`You must specify --dev or --local flag!`);
        console.log(`Example: node deploy-contract.js --local`);
        process.exit(-1);
    }

    const web3 = new Web3(web3Url);
    // let privateKey = process.env.DEPLOYER_PRIVATE_KEY || DEPLOYER_PRIVATE_KEY;
    // let account = web3.eth.accounts.privateKeyToAccount('0x' + privateKey);

    console.log('Forming transaction...');
    let rawTxBytes = [];
    let fuelTxHex = "000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000400000000000000000000000000000008000000000000000100000000000000010000000000000001240400000000000000000000000000000000000000000000c49d65de61cf04588a764b557d25cc6c6b4bc0d7429227e2a21e61c213b3a3e20000000000008212f1e92c42b90934aa6372e30bc568a326f6e66a1a0288595e6e3fbd392a4f3e6e000000000000006400000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000002123f2cafad611543e0265d89f1c2b60d9ebf5d56ad7e23d9827d6b522fd4d6e4000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000040c9dc5e3172da805511b064b544969ec725c64e242df8030f1b42855cbebf0fcaad691d6d6816d8199d532938b4c8e1a6f2db5dabb9711d79e8445446fa22fca0";
    let fuelTxBytes = Array.from(hexToBytes(fuelTxHex));
    let txType = 0x52; // FluentV1 tx
    let execEnv = 0x00; // fuel exec env
    let execEnvAndFuelTxBytes = [].concat(execEnv, fuelTxBytes);

    let rlpHeader = new RlpHeader(true, execEnvAndFuelTxBytes.length)
    console.log(`rlpHeader: ${JSON.stringify(rlpHeader)}`);

    rawTxBytes.push(txType);
    console.log(`rawTxBytes: ${rawTxBytes}`);
    // TODO append rlp header to rawTxBytes
    rawTxBytes.push(...rlpHeader.encode())
    console.log(`rawTxBytes: ${JSON.stringify(rawTxBytes)}`);
    rawTxBytes.push(...execEnvAndFuelTxBytes);

    let rawTxHex = "0x" + Buffer.from(rawTxBytes).toString("hex");
    console.log(`txHex ${rawTxHex}`)
    console.log(`rawTxBytes.length ${rawTxBytes.length}`)
    // const signedTransaction = await web3.eth.accounts.signTransaction(rawTx, privateKey)

    await ethRpcMethods.sendRawTransaction(web3.requestManager, rawTxHex);

    process.exit(0)
}

main().then(console.log).catch(console.error);
