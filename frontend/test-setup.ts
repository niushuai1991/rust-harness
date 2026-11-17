import '@testing-library/jest-dom/vitest';
import {configure} from '@testing-library/react';
// 全量并行跑测试时 CPU 争抢会让 findBy*/waitFor 的默认 1 秒超时偶发超限，报「找不到元素」的假失败。
// 放宽到 5 秒：命中即返回不受影响，只有真失败时才等满超时。
configure({asyncUtilTimeout: 5000});
