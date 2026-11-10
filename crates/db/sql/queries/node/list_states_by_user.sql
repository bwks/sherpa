SELECT node.lab, node.state
FROM node
JOIN lab ON lab.id = node.lab
WHERE lab.user = ?1
