import '@testing-library/jest-dom/vitest';
import {configure} from '@testing-library/react';
// 全量并行跑测试时 CPU 争抢会让 findBy*/waitFor 的默认 1 秒超时偶发超限，报「找不到元素」的假失败。
// 放宽到 5 秒：命中即返回不受影响，只有真失败时才等满超时。
// 必须明显小于 vitest 的 testTimeout（vite.config.ts，20 秒）：两者相等时 waitFor 失败会先耗尽
// 测试预算，真实断言错误被替换成无信息量的「Test timed out」。
configure({asyncUtilTimeout: 5000});
