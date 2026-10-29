// SPDX-License-Identifier: MPL-2.0
import assert from 'node:assert/strict';
import test from 'node:test';
import { resolve } from 'node:path';
import { cargoLegalSource } from '../apps/desktop/scripts/third-party-source.mjs';

test('legal sources distinguish a registry archive, exact Git revision, and local patch', () => {
  const root=resolve('fixture/project'), pkg={name:'sample',version:'1.2.3',manifest_path:resolve(root,'vendor/sample/Cargo.toml')};
  assert.deepEqual(cargoLegalSource(root,{...pkg,source:'registry+https://github.com/rust-lang/crates.io-index'}),{source:'https://crates.io/crates/sample/1.2.3',localSource:false});
  assert.deepEqual(cargoLegalSource(root,{...pkg,source:'git+https://example.invalid/source?rev=abc#abc'}),{source:'https://example.invalid/source?rev=abc#abc',localSource:false});
  assert.deepEqual(cargoLegalSource(root,{...pkg,source:null}),{source:'Mewu source tree: vendor/sample',localSource:true});
});

test('local legal locators never disclose paths outside the source tree', () => {
  const root=resolve('fixture/project');
  for(const manifest_path of [resolve(root,'../private/Cargo.toml'),resolve(root,'../../other/Cargo.toml')]){
    assert.throws(()=>cargoLegalSource(root,{name:'sample',version:'1.0',source:null,manifest_path}),/outside the project/);
  }
});
