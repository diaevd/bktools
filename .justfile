default:
        @just -l

publish *FLAGS:
        cargo release --tag-prefix='' --no-push {{FLAGS}}
