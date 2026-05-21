const { hello } = require('./hello');

test('hello returns greeting', () => {
  expect(hello('World')).toBe('Hello, World!');
});
