import {describe,expect,it} from 'vitest';
import {formatElapsed} from './elapsed';

describe('formatElapsed', () => {
 const start='2026-11-22T10:00:00Z';
 const at=(offsetSeconds:number)=>Date.parse(start)+offsetSeconds*1000;
 it('补零显示分秒', () => {
  expect(formatElapsed(start,at(0))).toBe('00:00');
  expect(formatElapsed(start,at(5))).toBe('00:05');
  expect(formatElapsed(start,at(65))).toBe('01:05');
  expect(formatElapsed(start,at(3599))).toBe('59:59');
 });
 it('跨小时进位为 h:mm:ss', () => {
  expect(formatElapsed(start,at(3600))).toBe('1:00:00');
  expect(formatElapsed(start,at(3700))).toBe('1:01:40');
 });
 it('时钟回拨与无效时间不产生负数或异常', () => {
  expect(formatElapsed(start,at(-10))).toBe('00:00');
  expect(formatElapsed('not-a-time',at(10))).toBe('--:--');
 });
});
