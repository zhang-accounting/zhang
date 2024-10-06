

interface Props {
  payee?: string;
  narration?: string;
}

export default function PayeeNarration(props: Props) {
  return (
    <div className='flex items-center gap-2'>
      {props.payee && (
        <span className="font-bold after:content-['·'] after:font-bold after:ml-2">
          {props.payee}
        </span>
      )}
      <span>{props.narration ?? ''}</span>
    </div>
  );
}
