// Greeter test — covers greet only, NOT farewell or formatName
import { greet } from '../src/greeter';

const result = greet('World');
console.assert(result === 'Hello, World!', 'greet works');
