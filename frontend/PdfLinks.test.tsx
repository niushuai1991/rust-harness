import {afterEach,expect,it} from 'vitest';
import {cleanup,fireEvent,render,screen} from '@testing-library/react';
import {SessionMarkdown,LinkedText} from './SessionLinks';
import {useSidePanel} from './sidePanelStore';

afterEach(()=>{cleanup();useSidePanel.getState().reset()});
it('点击 Markdown 本地 PDF 链接打开右侧 PDF 预览',()=>{
 render(<SessionMarkdown content={'[报告](D:/code/financial_agent/output/report.pdf)'}/>);
 fireEvent.click(screen.getByRole('link',{name:'报告'}));
 expect(useSidePanel.getState()).toMatchObject({open:true,view:'pdf',pdfPath:'D:/code/financial_agent/output/report.pdf'});
});
it('点击 file 链接解码本地 PDF 路径',()=>{
 render(<SessionMarkdown content={'[报告](file:///D:/my%20files/report.PDF)'}/>);
 fireEvent.click(screen.getByRole('link',{name:'报告'}));
 expect(useSidePanel.getState()).toMatchObject({view:'pdf',pdfPath:'D:/my files/report.PDF'});
});
it('用户文本中的绝对路径可以打开 PDF',()=>{
 render(<LinkedText content={'请查看 D:\\reports\\report.pdf。'}/>);
 fireEvent.click(screen.getByRole('link',{name:'D:\\reports\\report.pdf'}));
 expect(useSidePanel.getState()).toMatchObject({view:'pdf',pdfPath:'D:\\reports\\report.pdf'});
});
it('远程 PDF 仍使用网页面板',()=>{
 render(<SessionMarkdown content={'[报告](https://example.com/report.pdf)'}/>);
 fireEvent.click(screen.getByRole('link',{name:'报告'}));
 expect(useSidePanel.getState()).toMatchObject({view:'web',webUrl:'https://example.com/report.pdf'});
});

it('相对路径 PDF 链接在当前会话打开预览',()=>{
 render(<SessionMarkdown content={'[报告](output/report.pdf)'}/>);
 fireEvent.click(screen.getByRole('link',{name:'报告'}));
 expect(useSidePanel.getState()).toMatchObject({view:'pdf',pdfPath:'output/report.pdf'});
});

it('用户消息中的 Markdown 本地 PDF 链接可点击',()=>{
 render(<LinkedText content={'[报告](D:/reports/a.pdf)'}/>);
 fireEvent.click(screen.getByRole('link',{name:'D:/reports/a.pdf'}));
 expect(useSidePanel.getState().pdfPath).toBe('D:/reports/a.pdf');
});
it('用户消息中的带空格 Windows PDF 路径可点击',()=>{
 render(<LinkedText content={'请看 D:\\My Documents\\report.pdf。'}/>);
 fireEvent.click(screen.getByRole('link',{name:'D:\\My Documents\\report.pdf'}));
 expect(useSidePanel.getState().pdfPath).toBe('D:\\My Documents\\report.pdf');
});
