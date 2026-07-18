import { describe, expect, test } from 'bun:test';
import { createStoneMipChain, generateStonePage, pageFromStoneMipChain, sampleStoneBase, VT_PAGE_BORDER, VT_PAGE_SIZE, VT_SLOT_SIZE } from './procedural-vt.ts';

const pixel=(data:Uint8Array,x:number,y:number)=>[...data.slice((y*VT_SLOT_SIZE+x)*4,(y*VT_SLOT_SIZE+x)*4+4)];

describe('procedural virtual stone',()=>{
  test('is deterministic and wall seeds are unique',()=>{
    const a=generateStonePage(11,3,17,9), b=generateStonePage(11,3,17,9), c=generateStonePage(12,3,17,9);
    expect(a).toEqual(b);expect(a).not.toEqual(c);expect(a.byteLength).toBe(136*136*4);
  });
  test('neighbor borders reproduce identical absolute texels',()=>{
    for(const [seed,mip,x,y] of [[1,0,10,20],[97,4,31,7],[1234,9,1,0]]){
      const left=generateStonePage(seed,mip,x,y),right=generateStonePage(seed,mip,x+1,y);
      for(let row=0;row<VT_SLOT_SIZE;row++) for(let border=0;border<VT_PAGE_BORDER;border++)
        expect(pixel(left,VT_PAGE_BORDER+VT_PAGE_SIZE+border,row)).toEqual(pixel(right,VT_PAGE_BORDER+border,row));
    }
  });
  test('derives lower mips by 4x4 supersampling the original function',()=>{
    const seed=31,mip=3,pageX=1,pageY=1,page=generateStonePage(seed,mip,pageX,pageY),sum=[0,0,0],scale=2**mip,step=scale/4,baseX=pageX*VT_PAGE_SIZE*scale,baseY=pageY*VT_PAGE_SIZE*scale;
    for(let y=0;y<4;y++)for(let x=0;x<4;x++){
      const sample=sampleStoneBase(seed,baseX+(x+.5)*step,baseY+(y+.5)*step);
      for(let channel=0;channel<3;channel++)sum[channel]+=sample[channel];
    }
    const actual=pixel(page,VT_PAGE_BORDER,VT_PAGE_BORDER);
    for(let channel=0;channel<3;channel++)expect(actual[channel]).toBe(Math.floor(sum[channel]/16));
  });
  test('builds coarse pages from a recursive box-filtered chain',()=>{
    const chain=createStoneMipChain(5,1024,3),fine=chain.levels.get(3)!,coarse=chain.levels.get(4)!;
    for(let c=0;c<4;c++)expect(coarse.data[c]).toBe(Math.round((fine.data[c]+fine.data[4+c]+fine.data[fine.size*4+c]+fine.data[fine.size*4+4+c])/4));
    expect(pageFromStoneMipChain(chain,4,0,0).byteLength).toBe(VT_SLOT_SIZE*VT_SLOT_SIZE*4);
  });
  test('supports the complete 128K regular mip range',()=>{
    for(let mip=0;mip<=10;mip++) expect(generateStonePage(7,mip,0,0).byteLength).toBe(VT_SLOT_SIZE*VT_SLOT_SIZE*4);
  });
});
