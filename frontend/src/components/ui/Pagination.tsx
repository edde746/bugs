import Button from "~/components/ui/Button";

interface PaginationProps {
  onPrev: () => void;
  onNext: () => void;
  hasPrev: boolean;
  hasNext: boolean;
}

export default function Pagination(props: PaginationProps) {
  return (
    <div class="flex items-center gap-2">
      <Button
        variant="ghost"
        size="sm"
        disabled={!props.hasPrev}
        onClick={props.onPrev}
      >
        &larr; Prev
      </Button>
      <Button
        variant="ghost"
        size="sm"
        disabled={!props.hasNext}
        onClick={props.onNext}
      >
        Next &rarr;
      </Button>
    </div>
  );
}
