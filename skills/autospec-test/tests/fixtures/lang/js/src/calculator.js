// Calculator module — fixture for function-presence tests
'use strict';

/**
 * Add two numbers.
 * @param {number} a
 * @param {number} b
 * @returns {number}
 */
function add(a, b) {
  return a + b;
}

/**
 * Subtract b from a.
 * @param {number} a
 * @param {number} b
 * @returns {number}
 */
function subtract(a, b) {
  return a - b;
}

/**
 * Multiply two numbers.
 */
function multiply(a, b) {
  return a * b;
}

// Private helper — should NOT appear in exported list
function _internalHelper(x) {
  return x * 2;
}

module.exports = { add, subtract, multiply };
