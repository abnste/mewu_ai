// SPDX-License-Identifier: MPL-2.0
import { appearanceChoiceKey, appearanceChoices, mergeAppearanceRecords, type AppearanceRecord, type AppearanceSelection, type AppearanceChoice } from './appearance';
export interface AppearanceState { choices:AppearanceChoice[]; selection:AppearanceSelection|null; active:AppearanceChoice|null; ready:boolean }
export class AppearanceController {
  private records:AppearanceRecord[]=[];
  private selection:AppearanceSelection|null=null;
  private ready=false;
  private generation=0;
  private disposed=false;
  private stop:(()=>void)|undefined;
  constructor(private readonly api:{get:()=>Promise<AppearanceRecord[]>;subscribe:(accept:(records:AppearanceRecord[])=>void)=>Promise<()=>void>;apply:(choice:AppearanceChoice|null)=>void;publish:(state:AppearanceState)=>void}){}
  select(selection:AppearanceSelection|null){if(this.disposed)return;this.selection=selection;this.render();}
  private render(){const choices=appearanceChoices(this.records),key=this.selection&&appearanceChoiceKey(this.selection),active=choices.find(c=>appearanceChoiceKey(c)===key)??null;this.api.apply(active);this.api.publish({choices,selection:this.selection,active,ready:this.ready});}
  async initialize(){
    let events=0;const generation=++this.generation;
    const accept=(records:AppearanceRecord[])=>{if(this.disposed||generation!==this.generation)return;this.records=mergeAppearanceRecords(this.records,records);this.ready=true;this.render();};
    try{
      const stop=await this.api.subscribe(records=>{events++;accept(records);});
      if(this.disposed||generation!==this.generation){stop();return;}this.stop=stop;
      const before=events,records=await this.api.get();if(before===events)accept(records);
    }catch{if(!this.disposed&&generation===this.generation){this.ready=true;this.render();}}
  }
  dispose(){this.disposed=true;this.generation++;this.stop?.();this.api.apply(null);}
}
