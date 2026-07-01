import { AppIcon } from "../../ui-icons.jsx";

export function ComingSoon({ title, description }) {
  return (
    <section className="panel active">
      <div className="panel-header">
        <h1>{title}</h1>
      </div>
      <div className="coming-soon">
        <div className="coming-icon"><AppIcon name="brain" size={64} /></div>
        <h2>COMING SOON</h2>
        <p>{description}</p>
      </div>
    </section>
  );
}

export function EmptyItem({ text }) {
  return <li className="empty">{text}</li>;
}
