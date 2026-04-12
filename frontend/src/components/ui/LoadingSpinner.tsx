export default function LoadingSpinner(props: { class?: string }) {
  return (
    <div class={`spinner ${props.class ?? ""}`}>
      <div class="spinner__circle" />
    </div>
  );
}
