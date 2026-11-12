import { createServer } from 'node:http';
import { isIP } from 'node:net';
import { ApiError, need } from './contracts.mjs';

export function httpServer(api,{origin,trustProxy=false,assets=new Map()}={}) {
  const server=createServer(async(req,res)=>{
    res.setHeader('X-Content-Type-Options','nosniff');
    res.setHeader('Cache-Control','no-store');
    res.setHeader('Referrer-Policy','no-referrer');
    res.setHeader('Content-Security-Policy',"default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'");
    try {
      need(['GET','POST','PUT','DELETE'].includes(req.method),'method_not_allowed',405);
      const url=new URL(req.url,'http://localhost');
      need(!req.headers.origin || req.headers.origin===origin,'origin_not_allowed',403);
      if(req.method==='GET' && assets.has(url.pathname)) {
        const asset=assets.get(url.pathname); res.setHeader('Content-Type',asset.type); res.end(asset.bytes); return;
      }
      need(url.pathname.startsWith('/v1/'),'not_found',404);
      let body={};
      if(req.method!=='GET') {
        need(/^application\/json(?:\s*;|$)/i.test(req.headers['content-type'] || ''),'json_required',415);
        need(!req.headers['content-encoding'],'encoding_not_supported',415);
        need(!req.headers['content-length'] || Number(req.headers['content-length'])<=65536,'request_too_large',413);
        const chunks=[]; let bytes=0;
        for await(const chunk of req) { bytes+=chunk.length; need(bytes<=65536,'request_too_large',413); chunks.push(chunk); }
        try { body=JSON.parse(Buffer.concat(chunks).toString('utf8')); } catch { throw new ApiError('invalid_json'); }
      }
      let ip=req.socket.remoteAddress || 'unknown';
      // Enable only behind the bundled loopback proxy, which OVERWRITES X-Real-IP.
      if(trustProxy) { const real=req.headers['x-real-ip']; need(typeof real==='string' && isIP(real),'proxy_header_required',400); ip=real; }
      const value=await api({method:req.method,url,body,ip,headers:req.headers});
      res.setHeader('Content-Type','application/json; charset=utf-8'); res.end(JSON.stringify(value));
    } catch(error) {
      res.statusCode=error instanceof ApiError?error.status:500;
      if(res.statusCode===429) res.setHeader('Retry-After','60');
      res.setHeader('Content-Type','application/json; charset=utf-8');
      res.end(JSON.stringify({error:error instanceof ApiError?error.message:'internal_error'}));
      if(!(error instanceof ApiError)) console.error('request_failed'); // Never log payload, email, token or OTP.
    }
  });
  server.requestTimeout=30000; server.headersTimeout=10000; server.keepAliveTimeout=5000; server.maxHeadersCount=32;
  return server;
}
