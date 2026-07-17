import React from "react";
function EmptyItem({ text }) { return <li className="empty">{text}</li>;
}
function CardHeader({ title, badge, kicker, className = "card-header", children }) { return ( <div className={className}>
      {kicker ? ( <div>
          <span className="analytics-card-kicker">{kicker}</span>
          <h2>{title}</h2>
        </div>
      ) : ( <h2>{title}</h2>
      )}
      {badge === void 0 ? null : <span className="badge">{badge}</span>}
      {children}
    </div> );
}
function ListCard({ title, badge, items, emptyText, renderItem, className = "card", listClassName = "item-list" }) { return ( <div className={className}>
      <CardHeader title={title} badge={badge ?? items.length} />
      <ul className={listClassName}>{items.length ? items.map(renderItem) : <EmptyItem text={emptyText} />}</ul>
    </div> );
}
function StatusRows({ rows, className = "overview-status-list", rowClassName = "overview-status-row" }) { return ( <div className={className}>
      {rows.map(({ label, value, key = label, title, valueClassName }) => ( <div key={key} className={rowClassName}>
          <span title={title}>{label}</span>
          <strong className={valueClassName}>{value}</strong>
        </div>
      ))}
    </div> );
}
function StatChip({ label, children }) { return ( <div className="analytics-stat-chip">
      <span className="analytics-stat-chip-label">{label}</span>
      <strong>{children}</strong>
    </div> );
}
function SurfaceStatGrid({ stats }) { return ( <div className="surface-stat-grid">
      {stats.map(({ label, value }) => ( <div key={label} className="surface-stat-card">
          <span className="surface-stat-label">{label}</span>
          <strong>{value}</strong>
        </div>
      ))}
    </div> );
}
export { CardHeader, EmptyItem, ListCard, StatChip, StatusRows, SurfaceStatGrid };
