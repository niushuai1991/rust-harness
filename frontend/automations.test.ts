import {describe,expect,it} from 'vitest';
import {AUTOMATION_KINDS,defaultOnceValue,fmtLocalInput,fmtSchedule,nextRunAt,parseHhmm} from './automations';
import type {ScheduleLike} from './automations';

const like=(over:Partial<ScheduleLike>):ScheduleLike=>({kind:'daily',at:null,time:'09:30',weekdays:null,monthDay:null,month:null,...over});
const at=(y:number,m:number,d:number,h:number,mi:number,s=0)=>new Date(y,m-1,d,h,mi,s);

describe('自动化展示纯函数',()=>{
 it('defaultOnceValue 默认为当天当前时:分且秒归零',()=>{
  expect(defaultOnceValue(at(2026,11,7,20,27,59))).toBe('2026-11-07T20:27:00');
  expect(defaultOnceValue(at(2026,1,2,3,4))).toBe('2026-01-02T03:04:00');
 });
 it('fmtLocalInput 保留秒用于编辑回填',()=>{
  expect(fmtLocalInput(at(2026,11,7,21,30,5))).toBe('2026-11-07T21:30:05');
 });
 it('fmtSchedule 输出五类周期中文描述',()=>{
  expect(fmtSchedule(like({kind:'once',at:'2026-11-07T09:30:00Z'}))).toMatch(/^单次 2026-11-07 \d{2}:\d{2}:\d{2}$/);
  expect(fmtSchedule(like({kind:'daily'}))).toBe('每天 09:30');
  expect(fmtSchedule(like({kind:'weekly',weekdays:[5,1]}))).toBe('每周一、五 09:30');
  expect(fmtSchedule(like({kind:'monthly',monthDay:31}))).toBe('每月 31 日 09:30');
  expect(fmtSchedule(like({kind:'yearly',month:12,monthDay:31}))).toBe('每年 12 月 31 日 09:30');
 });
 it('nextRunAt：daily 当天未过取当天，已过取明天',()=>{
  expect(nextRunAt(like({kind:'daily'}),at(2026,11,7,8,0))).toEqual(at(2026,11,7,9,30));
  expect(nextRunAt(like({kind:'daily'}),at(2026,11,7,9,30))).toEqual(at(2026,11,8,9,30));
 });
 it('nextRunAt：weekly 取最近命中日，忽略空列表',()=>{
  expect(nextRunAt(like({kind:'weekly',weekdays:[1,5]}),at(2026,11,7,20,0))).toEqual(at(2026,11,9,9,30));
  expect(nextRunAt(like({kind:'weekly',weekdays:[6]}),at(2026,11,7,8,0))).toEqual(at(2026,11,7,9,30));
  expect(nextRunAt(like({kind:'weekly',weekdays:[]}),at(2026,11,7,8,0))).toBeNull();
 });
 it('nextRunAt：monthly 短月无 31 日时跳过',()=>{
  expect(nextRunAt(like({kind:'monthly',monthDay:31}),at(2026,11,7,8,0))).toEqual(at(2026,12,31,9,30));
  expect(nextRunAt(like({kind:'monthly',monthDay:1}),at(2026,11,7,8,0))).toEqual(at(2026,12,1,9,30));
 });
 it('nextRunAt：yearly 平年无 2/29 时跳到下个闰年',()=>{
  expect(nextRunAt(like({kind:'yearly',month:2,monthDay:29}),at(2026,11,7,8,0))).toEqual(at(2028,2,29,9,30));
  expect(nextRunAt(like({kind:'yearly',month:12,monthDay:31}),at(2026,12,31,22,0))).toEqual(at(2027,12,31,9,30));
 });
 it('nextRunAt：once 返回固定时刻，缺字段返回 null',()=>{
  expect(nextRunAt(like({kind:'once',at:'2026-11-07T01:30:00Z'}),at(2026,11,7,8,0))).toEqual(new Date('2026-11-07T01:30:00Z'));
  expect(nextRunAt(like({kind:'daily',time:null}),at(2026,11,7,8,0))).toBeNull();
 });
 it('parseHhmm 校验 24 小时制边界',()=>{
  expect(parseHhmm('23:59')).toEqual([23,59]);
  expect(parseHhmm('24:00')).toBeNull();
  expect(parseHhmm('9:5')).toEqual([9,5]);
  expect(parseHhmm('')).toBeNull();
 });
 it('周期选项顺序与标签',()=>{
  expect(AUTOMATION_KINDS.map(k=>k.label)).toEqual(['单次','每天','每周','每月','每年']);
 });
});
