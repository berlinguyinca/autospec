// utils.mjs — utility functions

/**
 * Returns a greeting string.
 * @param {string} name
 * @returns {string}
 */
export function greet(name) {
  return `Hello, ${name}!`;
}

/**
 * Formats a date to ISO string.
 * @param {Date} date
 * @returns {string}
 */
export function formatDate(date) {
  return date.toISOString();
}
