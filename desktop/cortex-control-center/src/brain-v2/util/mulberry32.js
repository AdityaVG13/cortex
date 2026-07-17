function mulberry32(seed){let s=seed>>>0||1;return function(){s=s+1831565813>>>0;let t=s;return t=Math.imul(t^t>>>15,t|1),t^=t+Math.imul(t^t>>>7,t|61),((t^t>>>14)>>>0)/4294967296}}export{mulberry32};
