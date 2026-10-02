interface Props {
  percentage: string;
}

export default function BackgroundProgress(props: Props) {
  const percentage = parseFloat(props.percentage);
  let color = 'var(--chart-1)';

  if (percentage > 20) {
    color = 'var(--chart-3)';
  }
  if (percentage > 40) {
    color = 'var(--chart-5)';
  }
  if (percentage > 60) {
    color = 'var(--chart-4)';
  }
  if (percentage > 80) {
    color = 'var(--negative)';
  }

  return (
    <div
      className="progressbar"
      style={{
        width: `${percentage}%`,
        position: 'absolute',
        top: '95%',
        left: 0,
        bottom: '-1px',
        backgroundColor: color,
        zIndex: -999,
      }}
    ></div>
  );
}
