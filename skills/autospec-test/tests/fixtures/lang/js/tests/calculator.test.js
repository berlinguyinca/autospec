// Calculator test — fixture covers add and subtract, NOT multiply
'use strict';

const { add, subtract } = require('../src/calculator');

// Simple assertions (no test framework needed for fixture)
console.assert(add(1, 2) === 3, 'add(1,2)===3');
console.assert(subtract(5, 3) === 2, 'subtract(5,3)===2');
