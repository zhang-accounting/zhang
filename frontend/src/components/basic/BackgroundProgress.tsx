import * as React from 'react';

interface Props {
  percentage: string;
}

export default function BackgroundProgress(props: Props) {
  let percentage = parseFloat(props.percentage);
  let color = 'rgb(64,184,86)';

  if (percentage > 20) {
    color = 'rgb(36,130,245)';
  }
  if (percentage > 40) {
    color = 'rgb(123,3,123)';
  }
  if (percentage > 60) {
    color = 'rgb(244,160,10)';
  }
  if (percentage > 80) {
    color = 'rgb(244,10,6)';
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
