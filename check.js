let fs = require('fs');
let lines = fs.readFileSync('test.txt', 'utf-8').split('\n');
let firstLine = lines[0];
let splitAt = lines.indexOf(firstLine, 1);
let leftSide = lines.slice(0, splitAt);
let rightSide = lines.slice(splitAt);
console.assert(leftSide[0] === rightSide[0])

for (let i = 0; i <= leftSide.length; i++) {
    console.log(i + '\t' + leftSide[i]);
    if (leftSide[i] !== rightSide[i]) {
        console.log(' != ', rightSide[i]);
        throw new Error('MISMATCH');
    }
}