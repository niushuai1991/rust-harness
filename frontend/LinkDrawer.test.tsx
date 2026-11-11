import {act} from '@testing-library/react';
import {expect,it} from 'vitest';
import {useSidePanel} from './sidePanelStore';

it('打开网页时切换到网页视图并保留地址',()=>{
  act(()=>useSidePanel.getState().openWeb('https://example.com/path'));
  expect(useSidePanel.getState()).toMatchObject({open:true,view:'web',webUrl:'https://example.com/path'});
});
