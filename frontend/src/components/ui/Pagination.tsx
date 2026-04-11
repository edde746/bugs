import IconArrowLeft from "~icons/lucide/arrow-left";
import IconArrowRight from "~icons/lucide/arrow-right";
import Button from "~/components/ui/Button";

interface PaginationProps {
  onPrev: () => void;
  onNext: () => void;
  hasPrev: boolean;
  hasNext: boolean;
}

export default function Pagination(props: PaginationProps) {
  return (
    <div class="pagination">
      <Button
        variant="ghost"
        size="sm"
        disabled={!props.hasPrev}
        onClick={props.onPrev}
      >
        <IconArrowLeft /> Prev
      </Button>
      <Button
        variant="ghost"
        size="sm"
        disabled={!props.hasNext}
        onClick={props.onNext}
      >
        Next <IconArrowRight />
      </Button>
    </div>
  );
}
