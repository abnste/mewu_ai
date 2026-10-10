// SPDX-License-Identifier: MPL-2.0
export default function DrawingWidthSlider(props:{label:string;value:number;min:number;max:number;disabled:boolean;onChange:(value:number,submit:boolean)=>void}) {
  return <label class="drawing-width-slider" title={props.label}>
    <input type="range" aria-label={props.label} min={props.min} max={Math.max(props.max,props.value)} step="1" value={props.value} disabled={props.disabled}
      style={{'--drawing-range-progress':`${(props.value-props.min)/(Math.max(props.max,props.value)-props.min)*100}%`}}
      onInput={event=>props.onChange(Number(event.currentTarget.value),false)} onChange={event=>props.onChange(Number(event.currentTarget.value),true)}/>
    <output>{props.value}<span>px</span></output>
  </label>;
}
