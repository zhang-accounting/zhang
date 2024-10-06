import { PluginResponse } from '../rest-model';


interface Props extends PluginResponse {}

export default function PluginBox(props: Props) {
  return (
    <div className="bg-transparent p-4 border-2 border-gray-100 rounded-md">
      <div className="flex justify-between items-center mb-2">
        <span className="text-lg font-semibold">{props.name}</span>
        <span className="bg-blue-500 text-white px-2 py-1 rounded-full text-sm">{props.version}</span>
      </div>
      <div className="flex flex-wrap gap-2">
        {props.plugin_type.map((item, index) => (
          <span key={index} className="bg-gray-200 text-gray-700 px-2 py-1 rounded-full text-sm">{item}</span>
        ))}
      </div>
    </div>
  );
}
